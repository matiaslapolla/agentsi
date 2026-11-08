//! The speech of the pill.
//!
//! One thread renders each reply one sentence at a time and puts the samples in a
//! queue. The output stream plays from the queue, so the first sentence plays while
//! the next one renders. A stop empties the queue at once: the stream plays silence
//! from its next buffer, and the thread drops the jobs that the stop made old.

use crate::link::{LinkEvent, LinkSender};
use crate::state::{Effect, Input, SpeechEvent};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The rate of the voice.
pub const VOICE_RATE: u32 = agentsi_voice::tts::RATE;
/// How often the thread looks whether the queue played to its end.
const DRAIN_POLL: Duration = Duration::from_millis(10);
/// The samples that one lock of the queue adds, so the stream never waits long for the lock.
const PUSH_PIECE: usize = 4096;
/// The buffer that the pill asks of the stream: 20 ms. A stop is heard within about one buffer.
const BUFFER_SECONDS: f32 = 0.02;
/// How long the stream keeps running after the queue is empty. The device holds up to
/// about 2 buffers, and a pause keeps them: the end of the reply would be cut, and would
/// play at the next start. After this time the device holds only silence.
const TAIL: Duration = Duration::from_millis(60);
/// The line that the pill shows when the output device goes away.
pub const OUTPUT_LOST: &str = "The audio output stopped";

/// Makes the samples of a reply.
pub trait Engine {
    /// The sentences of one `voice.speak` text, in order.
    fn sentences(&self, text: &str) -> Vec<String>;
    /// The voice of a role, and one warning for each fallback.
    fn voice_for(&self, role: &str) -> (String, Vec<String>);
    /// One sentence at [`VOICE_RATE`].
    fn render(&mut self, voice: &str, sentence: &str) -> Result<Vec<f32>, String>;
}

impl Engine for agentsi_voice::tts::Voice {
    fn sentences(&self, text: &str) -> Vec<String> {
        agentsi_voice::tts::Voice::sentences(self, text)
    }

    fn voice_for(&self, role: &str) -> (String, Vec<String>) {
        agentsi_voice::tts::Voice::voice_for(self, role)
    }

    fn render(&mut self, voice: &str, sentence: &str) -> Result<Vec<f32>, String> {
        agentsi_voice::tts::Voice::render(self, voice, sentence).map_err(|e| e.to_string())
    }
}

/// The stream that plays the queue.
pub trait Output {
    fn rate(&self) -> u32;
    fn play(&mut self) -> Result<(), String>;
    fn pause(&mut self);
}

pub type OpenEngine = Box<dyn FnOnce() -> Result<Box<dyn Engine>, String> + Send>;
pub type OpenOutput = Box<dyn Fn(Arc<Shared>) -> Result<Box<dyn Output>, String> + Send>;

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos())
}

/// What the speech thread, the window thread and the stream share.
#[derive(Default)]
pub struct Shared {
    /// Grows at each stop. A job of an older epoch is dropped.
    epoch: AtomicU64,
    /// Mono samples at the rate of the stream.
    queue: Mutex<VecDeque<f32>>,
    /// The stream reported an error, so the next job opens a new one.
    broken: AtomicBool,
    /// A new sound starts, for the timing line of `AGENTSI_PILL_STATS`.
    fresh: AtomicBool,
    /// A stop emptied the queue, for the timing line of `AGENTSI_PILL_STATS`.
    silenced: AtomicBool,
    stats: bool,
}

impl Shared {
    pub fn new(stats: bool) -> Shared {
        Shared {
            stats,
            ..Shared::default()
        }
    }

    fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    fn queue(&self) -> std::sync::MutexGuard<'_, VecDeque<f32>> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn queued(&self) -> usize {
        self.queue().len()
    }

    /// Adds samples of the job of `epoch`, after `gap` zeros when sound still plays.
    /// `false` when a stop made the job old.
    fn push(&self, epoch: u64, gap: usize, samples: &[f32]) -> bool {
        {
            let mut queue = self.queue();
            if self.epoch() != epoch {
                return false;
            }
            if queue.is_empty() {
                self.fresh.store(true, Ordering::SeqCst);
            } else {
                queue.extend(std::iter::repeat_n(0.0, gap));
            }
        }
        for piece in samples.chunks(PUSH_PIECE) {
            let mut queue = self.queue();
            if self.epoch() != epoch {
                return false;
            }
            queue.extend(piece);
        }
        true
    }

    fn stop(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.queue().clear();
        self.silenced.store(true, Ordering::SeqCst);
    }

    fn clear(&self) {
        self.queue().clear();
    }

    /// Fills one buffer of the stream: each frame gets the next sample on every channel,
    /// or silence when the queue is empty.
    pub fn fill<T: Sample + FromSample<f32>>(&self, out: &mut [T], channels: usize) {
        let mut sound = false;
        {
            let mut queue = self.queue();
            for frame in out.chunks_mut(channels.max(1)) {
                let s = queue.pop_front().unwrap_or(0.0);
                sound |= s != 0.0;
                frame.fill(T::from_sample(s));
            }
        }
        if self.stats && sound && self.fresh.swap(false, Ordering::SeqCst) {
            eprintln!("agentsi-pill: first sound at {}", unix_nanos());
        }
        self.mark_silence();
    }

    /// The first buffer after a stop, or the pause of the stream, whichever comes first.
    fn mark_silence(&self) {
        if self.silenced.swap(false, Ordering::SeqCst) && self.stats {
            eprintln!("agentsi-pill: silence at {}", unix_nanos());
        }
    }
}

struct Job {
    ticket: u64,
    epoch: u64,
    role: String,
    text: String,
}

/// The speech of the pill. The window thread calls it for each effect.
pub struct Speaker {
    jobs: Mutex<mpsc::Sender<Job>>,
    shared: Arc<Shared>,
}

impl Speaker {
    /// Starts the speech thread. It opens the engine at once, because that takes seconds,
    /// and the output stream at the first job.
    pub fn start(
        open_engine: OpenEngine,
        open_output: OpenOutput,
        events: LinkSender,
        stats: bool,
    ) -> Speaker {
        let shared = Arc::new(Shared::new(stats));
        let (tx, rx) = mpsc::channel();
        let thread_shared = shared.clone();
        std::thread::Builder::new()
            .name("speech".into())
            .spawn(move || {
                Worker {
                    shared: thread_shared,
                    jobs: rx,
                    events,
                    open_output,
                    output: None,
                }
                .run(open_engine)
            })
            .expect("the OS starts a thread");
        Speaker {
            jobs: Mutex::new(tx),
            shared,
        }
    }

    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    pub fn speak(&self, ticket: u64, role: &str, text: &str) {
        let job = Job {
            ticket,
            epoch: self.shared.epoch(),
            role: role.to_owned(),
            text: text.to_owned(),
        };
        let _ = self
            .jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(job);
    }

    /// Silence from the next buffer of the stream, and no more sound from the jobs so far.
    pub fn stop(&self) {
        self.shared.stop();
    }

    pub fn effect(&self, effect: &Effect) {
        match effect {
            Effect::StartSpeech {
                ticket, role, text, ..
            } => self.speak(*ticket, role, text),
            Effect::StopSpeech { .. } => self.stop(),
            Effect::StopRecording
            | Effect::StartRecording { .. }
            | Effect::FinishRecording { .. }
            | Effect::CancelRecording
            | Effect::Listen { .. }
            | Effect::EndConversation => {}
        }
    }
}

struct Worker {
    shared: Arc<Shared>,
    jobs: mpsc::Receiver<Job>,
    events: LinkSender,
    open_output: OpenOutput,
    output: Option<Box<dyn Output>>,
}

impl Worker {
    fn report(&self, ticket: u64, event: SpeechEvent) {
        let _ = self
            .events
            .unbounded_send(LinkEvent::Input(Input::Speech { ticket, event }));
    }

    fn run(mut self, open_engine: OpenEngine) {
        let opened = std::time::Instant::now();
        let mut engine = open_engine();
        match &engine {
            Ok(_) => eprintln!("agentsi-pill: voice ready in {:?}", opened.elapsed()),
            Err(e) => eprintln!("agentsi-pill: the voice cannot start: {e}"),
        }
        let mut next: Option<Job> = None;
        loop {
            let job = match next.take() {
                Some(job) => job,
                None => match self.idle() {
                    Some(job) => job,
                    None => return,
                },
            };
            if job.epoch != self.shared.epoch() {
                continue;
            }
            let engine = match &mut engine {
                Ok(engine) => engine,
                Err(e) => {
                    self.report(job.ticket, SpeechEvent::Failed(e.clone()));
                    continue;
                }
            };
            match self.speak(engine.as_mut(), &job) {
                Ok(true) => next = self.drain(&job),
                Ok(false) => {}
                Err(e) => {
                    eprintln!("agentsi-pill: the speech failed: {e}");
                    // The sentences before the failure must not play on, and the stream must pause.
                    self.shared.clear();
                    self.report(job.ticket, SpeechEvent::Failed(e));
                }
            }
        }
    }

    /// Waits for the next job. With an empty queue, the stream plays silence for `TAIL`
    /// and then pauses. `None` when the speaker is gone.
    fn idle(&mut self) -> Option<Job> {
        if self.output.is_some() && self.shared.queued() == 0 {
            match self.jobs.recv_timeout(TAIL) {
                Ok(job) => return Some(job),
                Err(RecvTimeoutError::Disconnected) => return None,
                Err(RecvTimeoutError::Timeout) => {}
            }
            if let Some(output) = self.output.as_mut() {
                output.pause();
            }
            self.shared.mark_silence();
        }
        self.jobs.recv().ok()
    }

    fn output(&mut self) -> Result<&mut Box<dyn Output>, String> {
        if self.shared.broken.swap(false, Ordering::SeqCst) {
            self.output = None;
            self.shared.clear();
        }
        if self.output.is_none() {
            self.output = Some((self.open_output)(self.shared.clone())?);
        }
        Ok(self.output.as_mut().expect("set above"))
    }

    /// Renders and queues each sentence of `job`. `Ok(false)` when a stop made it old.
    fn speak(&mut self, engine: &mut dyn Engine, job: &Job) -> Result<bool, String> {
        let rate = self.output()?.rate();
        let (voice, warnings) = engine.voice_for(&job.role);
        for w in warnings {
            eprintln!("agentsi-pill: {w}");
        }
        let gap = (agentsi_voice::tts::SENTENCE_GAP * rate as f32) as usize;
        let mut playing = false;
        for sentence in engine.sentences(&job.text) {
            if self.shared.epoch() != job.epoch {
                return Ok(false);
            }
            let samples = engine.render(&voice, &sentence)?;
            if samples.is_empty() {
                continue;
            }
            let samples = resample(&samples, VOICE_RATE, rate);
            if !self.shared.push(job.epoch, gap, &samples) {
                return Ok(false);
            }
            if !playing {
                playing = true;
                self.output()?.play()?;
                self.report(job.ticket, SpeechEvent::Playing);
            }
        }
        Ok(true)
    }

    /// Waits until the queue played to its end, then reports `Done`. A new job that comes
    /// first is returned, so it renders while the queue still plays.
    fn drain(&mut self, job: &Job) -> Option<Job> {
        match self.jobs.try_recv() {
            Ok(next) => return Some(next),
            Err(TryRecvError::Disconnected) => return None,
            Err(TryRecvError::Empty) => {}
        }
        loop {
            if self.shared.epoch() != job.epoch {
                return None;
            }
            // A lost device calls the error callback once and never asks for samples again.
            if self.shared.broken.load(Ordering::SeqCst) {
                self.shared.clear();
                self.report(job.ticket, SpeechEvent::Failed(OUTPUT_LOST.into()));
                return None;
            }
            if self.shared.queued() == 0 {
                return match self.jobs.recv_timeout(TAIL) {
                    Ok(next) => Some(next),
                    Err(RecvTimeoutError::Disconnected) => None,
                    Err(RecvTimeoutError::Timeout) => {
                        self.report(job.ticket, SpeechEvent::Done);
                        None
                    }
                };
            }
            match self.jobs.recv_timeout(DRAIN_POLL) {
                Ok(next) => return Some(next),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return None,
            }
        }
    }
}

/// Linear resampling. Kokoro speech has little above 8 kHz, so this is enough for 24 to 48 kHz.
pub fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let len = (samples.len() as u64 * u64::from(to) / u64::from(from)) as usize;
    let step = f64::from(from) / f64::from(to);
    (0..len)
        .map(|i| {
            let x = i as f64 * step;
            let k = x as usize;
            let t = (x - k as f64) as f32;
            let a = samples[k.min(samples.len() - 1)];
            let b = samples[(k + 1).min(samples.len() - 1)];
            a + (b - a) * t
        })
        .collect()
}

/// Opens the Kokoro voice with the standard folders and `espeak-ng`.
pub fn open_voice() -> OpenEngine {
    Box::new(|| {
        agentsi_voice::tts::Voice::open(agentsi_voice::tts::Options::standard())
            .map(|v| Box::new(v) as Box<dyn Engine>)
            .map_err(|e| e.to_string())
    })
}

struct CpalOutput {
    stream: cpal::Stream,
    rate: u32,
}

impl Output for CpalOutput {
    fn rate(&self) -> u32 {
        self.rate
    }

    fn play(&mut self) -> Result<(), String> {
        self.stream.play().map_err(|e| e.to_string())
    }

    fn pause(&mut self) {
        // A host that cannot pause plays silence instead.
        let _ = self.stream.pause();
    }
}

/// The sample formats that the pill writes, in order of preference.
const FORMATS: [SampleFormat; 8] = [
    SampleFormat::F32,
    SampleFormat::I16,
    SampleFormat::I32,
    SampleFormat::U16,
    SampleFormat::F64,
    SampleFormat::U8,
    SampleFormat::I8,
    SampleFormat::U32,
];

/// A stream at 24 kHz if the device takes it, else at its default rate. `f32` first.
fn pick(device: &cpal::Device) -> Result<(StreamConfig, SampleFormat), String> {
    let configs: Vec<_> = device
        .supported_output_configs()
        .map_err(|e| e.to_string())?
        .filter(|c| FORMATS.contains(&c.sample_format()))
        .collect();
    let at_voice_rate = FORMATS.iter().find_map(|f| {
        configs
            .iter()
            .filter(|c| c.sample_format() == *f)
            .find_map(|c| (*c).try_with_sample_rate(VOICE_RATE))
    });
    let supported = match at_voice_rate {
        Some(c) => c,
        None => device.default_output_config().map_err(|e| e.to_string())?,
    };
    let mut config = supported.config();
    let want = (supported.sample_rate() as f32 * BUFFER_SECONDS) as u32;
    if let cpal::SupportedBufferSize::Range { min, max } = supported.buffer_size() {
        config.buffer_size = cpal::BufferSize::Fixed(want.clamp(*min, *max));
    }
    Ok((config, supported.sample_format()))
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let on_error = shared.clone();
    device
        .build_output_stream(
            *config,
            move |out: &mut [T], _| shared.fill(out, channels),
            move |e| {
                eprintln!("agentsi-pill: the output stream failed: {e}");
                on_error.broken.store(true, Ordering::SeqCst);
            },
            None,
        )
        .map_err(|e| e.to_string())
}

/// Opens the output device that `wanted` names. The stream starts paused.
pub fn open_cpal(wanted: Option<String>) -> OpenOutput {
    Box::new(move |shared| {
        let device = crate::audio::output_device(wanted.as_deref())?;
        let (config, format) = pick(&device)?;
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, &config, shared),
            SampleFormat::I16 => build::<i16>(&device, &config, shared),
            SampleFormat::I32 => build::<i32>(&device, &config, shared),
            SampleFormat::U16 => build::<u16>(&device, &config, shared),
            SampleFormat::F64 => build::<f64>(&device, &config, shared),
            SampleFormat::U8 => build::<u8>(&device, &config, shared),
            SampleFormat::I8 => build::<i8>(&device, &config, shared),
            SampleFormat::U32 => build::<u32>(&device, &config, shared),
            other => Err(format!("the output device wants samples of type {other}")),
        }?;
        let _ = stream.pause();
        if crate::stats_enabled() {
            eprintln!(
                "agentsi-pill: output {} Hz, {} channels, {:?}, buffer {:?}",
                config.sample_rate, config.channels, format, config.buffer_size
            );
        }
        Ok(Box::new(CpalOutput {
            stream,
            rate: config.sample_rate,
        }) as Box<dyn Output>)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::channel::mpsc::{UnboundedReceiver, unbounded};
    use std::sync::atomic::AtomicUsize;

    /// Renders each sentence as 2400 samples of its number. It waits for `gate`
    /// before each sentence after the first, and records the queue length at each render.
    struct Fake {
        renders: Arc<Mutex<Vec<(String, usize)>>>,
        shared: Arc<Mutex<Option<Arc<Shared>>>>,
        gate: Arc<(Mutex<usize>, std::sync::Condvar)>,
        fail_on: Option<&'static str>,
    }

    impl Engine for Fake {
        fn sentences(&self, text: &str) -> Vec<String> {
            text.lines().map(str::to_owned).collect()
        }

        fn voice_for(&self, role: &str) -> (String, Vec<String>) {
            (format!("voice-of-{role}"), Vec::new())
        }

        fn render(&mut self, _voice: &str, sentence: &str) -> Result<Vec<f32>, String> {
            if self.fail_on == Some(sentence) {
                return Err(format!("cannot say {sentence}"));
            }
            let queued = self
                .shared
                .lock()
                .unwrap()
                .as_ref()
                .map_or(0, |s| s.queued());
            let n = {
                let mut renders = self.renders.lock().unwrap();
                renders.push((sentence.to_owned(), queued));
                renders.len()
            };
            if n > 1 {
                let (lock, cv) = &*self.gate;
                let mut open = lock.lock().unwrap();
                while *open < n {
                    open = cv.wait(open).unwrap();
                }
            }
            Ok(vec![n as f32; 2400])
        }
    }

    struct Sink {
        plays: Arc<AtomicUsize>,
        pauses: Arc<AtomicUsize>,
    }

    impl Output for Sink {
        fn rate(&self) -> u32 {
            VOICE_RATE
        }
        fn play(&mut self) -> Result<(), String> {
            self.plays.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn pause(&mut self) {
            self.pauses.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct Rig {
        speaker: Speaker,
        events: UnboundedReceiver<LinkEvent>,
        renders: Arc<Mutex<Vec<(String, usize)>>>,
        gate: Arc<(Mutex<usize>, std::sync::Condvar)>,
        plays: Arc<AtomicUsize>,
        pauses: Arc<AtomicUsize>,
    }

    fn rig(fail_on: Option<&'static str>, engine_error: Option<&'static str>) -> Rig {
        let renders = Arc::new(Mutex::new(Vec::new()));
        let shared_slot = Arc::new(Mutex::new(None));
        let gate = Arc::new((Mutex::new(1), std::sync::Condvar::new()));
        let plays = Arc::new(AtomicUsize::new(0));
        let pauses = Arc::new(AtomicUsize::new(0));
        let fake = Fake {
            renders: renders.clone(),
            shared: shared_slot.clone(),
            gate: gate.clone(),
            fail_on,
        };
        let open_engine: OpenEngine = Box::new(move || match engine_error {
            Some(e) => Err(e.to_owned()),
            None => Ok(Box::new(fake) as Box<dyn Engine>),
        });
        let sink_plays = plays.clone();
        let sink_pauses = pauses.clone();
        let open_output: OpenOutput = Box::new(move |shared| {
            *shared_slot.lock().unwrap() = Some(shared);
            Ok(Box::new(Sink {
                plays: sink_plays.clone(),
                pauses: sink_pauses.clone(),
            }) as Box<dyn Output>)
        });
        let (tx, events) = unbounded();
        Rig {
            speaker: Speaker::start(open_engine, open_output, tx, false),
            events,
            renders,
            gate,
            plays,
            pauses,
        }
    }

    impl Rig {
        fn open_gate(&self, upto: usize) {
            let (lock, cv) = &*self.gate;
            *lock.lock().unwrap() = upto;
            cv.notify_all();
        }

        fn next_event(&mut self) -> Input {
            let start = std::time::Instant::now();
            loop {
                if let Ok(event) = self.events.try_recv() {
                    match event {
                        LinkEvent::Input(input) => return input,
                        other => panic!("not a speech event: {other:?}"),
                    }
                }
                assert!(start.elapsed() < Duration::from_secs(5), "no speech event");
                std::thread::sleep(Duration::from_millis(2));
            }
        }

        /// Plays what is queued, as the stream would.
        fn play_all(&self) -> Vec<f32> {
            let mut out = vec![0.0f32; self.speaker.shared().queued()];
            self.speaker.shared().fill(&mut out, 1);
            out
        }
    }

    fn wait_for(cond: impl Fn() -> bool) {
        let start = std::time::Instant::now();
        while !cond() {
            assert!(start.elapsed() < Duration::from_secs(5), "timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn the_first_sentence_plays_before_the_last_renders() {
        let mut rig = rig(None, None);
        rig.speaker.speak(1, "pm", "one\ntwo\nthree");
        assert_eq!(
            rig.next_event(),
            Input::Speech {
                ticket: 1,
                event: SpeechEvent::Playing
            }
        );
        // Sentence two waits at the gate, and sentence one is already in the queue.
        wait_for(|| rig.renders.lock().unwrap().len() == 2);
        assert_eq!(rig.speaker.shared().queued(), 2400);
        assert_eq!(rig.plays.load(Ordering::SeqCst), 1);
        rig.open_gate(3);
        wait_for(|| rig.renders.lock().unwrap().len() == 3);
        let renders = rig.renders.lock().unwrap().clone();
        assert_eq!(renders[2].0, "three");
        assert!(
            renders[2].1 > 0,
            "sound is queued while the last sentence renders"
        );
        wait_for(|| rig.speaker.shared().queued() >= 3 * 2400);
        let played = rig.play_all();
        let gap = (agentsi_voice::tts::SENTENCE_GAP * VOICE_RATE as f32) as usize;
        assert_eq!(played.len(), 3 * 2400 + 2 * gap);
        assert_eq!(played[0], 1.0);
        assert_eq!(played[2400], 0.0);
        assert_eq!(played[2400 + gap], 2.0);
        assert_eq!(
            rig.next_event(),
            Input::Speech {
                ticket: 1,
                event: SpeechEvent::Done
            }
        );
    }

    #[test]
    fn a_stop_empties_the_queue_and_drops_the_rest_of_the_reply() {
        let mut rig = rig(None, None);
        rig.speaker.speak(1, "pm", "one\ntwo\nthree");
        rig.next_event();
        wait_for(|| rig.renders.lock().unwrap().len() == 2);
        rig.speaker.stop();
        assert_eq!(rig.speaker.shared().queued(), 0);
        let mut buffer = [1.0f32; 64];
        rig.speaker.shared().fill(&mut buffer, 2);
        assert!(buffer.iter().all(|s| *s == 0.0), "silence at once");
        rig.open_gate(10);
        // A job sent before the stop and still waiting is old too.
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(rig.speaker.shared().queued(), 0);
        assert_eq!(rig.renders.lock().unwrap().len(), 2, "three never renders");
        // A new reply after the stop plays.
        rig.speaker.speak(2, "pm", "four");
        assert_eq!(
            rig.next_event(),
            Input::Speech {
                ticket: 2,
                event: SpeechEvent::Playing
            }
        );
    }

    #[test]
    fn two_replies_play_one_after_the_other() {
        let mut rig = rig(None, None);
        rig.open_gate(10);
        rig.speaker.speak(1, "pm", "one");
        rig.speaker.speak(2, "pm", "two");
        let mut seen = Vec::new();
        while seen.len() < 3 {
            seen.push(rig.next_event());
            if rig.speaker.shared().queued() >= 2 * 2400 {
                rig.play_all();
            }
        }
        assert_eq!(
            seen,
            vec![
                Input::Speech {
                    ticket: 1,
                    event: SpeechEvent::Playing
                },
                Input::Speech {
                    ticket: 2,
                    event: SpeechEvent::Playing
                },
                Input::Speech {
                    ticket: 2,
                    event: SpeechEvent::Done
                },
            ]
        );
    }

    #[test]
    fn a_lost_output_device_fails_the_reply_and_drops_its_samples() {
        let mut rig = rig(None, None);
        rig.open_gate(10);
        rig.speaker.speak(1, "pm", "one");
        rig.next_event();
        // The stream reported an error and never asks for samples again.
        rig.speaker.shared().broken.store(true, Ordering::SeqCst);
        assert_eq!(
            rig.next_event(),
            Input::Speech {
                ticket: 1,
                event: SpeechEvent::Failed(OUTPUT_LOST.into())
            }
        );
        assert_eq!(rig.speaker.shared().queued(), 0);
        rig.speaker.speak(2, "pm", "two");
        rig.next_event();
        wait_for(|| rig.speaker.shared().queued() > 0);
        assert!(
            rig.play_all().iter().all(|s| *s == 2.0),
            "only the new reply plays"
        );
    }

    #[test]
    fn a_failed_sentence_drops_the_queue_and_pauses_the_stream() {
        let mut rig = rig(Some("two"), None);
        rig.speaker.speak(1, "pm", "one\ntwo");
        rig.next_event();
        assert_eq!(
            rig.next_event(),
            Input::Speech {
                ticket: 1,
                event: SpeechEvent::Failed("cannot say two".into())
            }
        );
        assert_eq!(rig.speaker.shared().queued(), 0);
        wait_for(|| rig.pauses.load(Ordering::SeqCst) == 1);
    }

    #[test]
    fn the_stream_runs_on_for_the_tail_before_done_and_the_pause() {
        let mut rig = rig(None, None);
        rig.speaker.speak(1, "pm", "one");
        rig.next_event();
        rig.play_all();
        let emptied = std::time::Instant::now();
        assert_eq!(
            rig.next_event(),
            Input::Speech {
                ticket: 1,
                event: SpeechEvent::Done
            }
        );
        assert!(
            emptied.elapsed() >= TAIL - DRAIN_POLL,
            "{:?}",
            emptied.elapsed()
        );
        assert_eq!(rig.pauses.load(Ordering::SeqCst), 0);
        wait_for(|| rig.pauses.load(Ordering::SeqCst) == 1);
        assert!(emptied.elapsed() >= 2 * TAIL - DRAIN_POLL);
    }

    #[test]
    fn a_render_error_is_reported() {
        let mut rig = rig(Some("bad"), None);
        rig.speaker.speak(1, "pm", "bad");
        assert_eq!(
            rig.next_event(),
            Input::Speech {
                ticket: 1,
                event: SpeechEvent::Failed("cannot say bad".into())
            }
        );
    }

    #[test]
    fn an_engine_that_cannot_open_fails_each_reply() {
        let mut rig = rig(None, Some("espeak-ng is not installed"));
        for ticket in [1, 2] {
            rig.speaker.speak(ticket, "pm", "one");
            assert_eq!(
                rig.next_event(),
                Input::Speech {
                    ticket,
                    event: SpeechEvent::Failed("espeak-ng is not installed".into())
                }
            );
        }
    }

    #[test]
    fn effects_map_to_speak_and_stop() {
        let mut rig = rig(None, None);
        rig.speaker.effect(&Effect::StartSpeech {
            ticket: 7,
            terminal: "t1".into(),
            role: "pm".into(),
            text: "one".into(),
        });
        assert_eq!(
            rig.next_event(),
            Input::Speech {
                ticket: 7,
                event: SpeechEvent::Playing
            }
        );
        let before = rig.speaker.shared().epoch();
        rig.speaker.effect(&Effect::StopSpeech { terminal: None });
        assert_eq!(rig.speaker.shared().epoch(), before + 1);
        assert_eq!(rig.speaker.shared().queued(), 0);
    }

    #[test]
    fn resample_doubles_the_rate_and_keeps_the_shape() {
        let up = resample(&[0.0, 1.0, 0.0], 24_000, 48_000);
        assert_eq!(up.len(), 6);
        assert_eq!(&up[..4], &[0.0, 0.5, 1.0, 0.5]);
        assert_eq!(resample(&[0.3], 24_000, 24_000), vec![0.3]);
        assert!(resample(&[], 24_000, 44_100).is_empty());
    }

    #[test]
    fn fill_writes_each_sample_on_every_channel() {
        let shared = Shared::new(false);
        assert!(shared.push(0, 0, &[0.5, -0.5]));
        let mut out = [0i16; 6];
        shared.fill(&mut out, 2);
        assert_eq!(out[0], out[1]);
        assert!(out[0] > 16_000);
        assert!(out[2] < -16_000);
        assert_eq!(&out[4..], &[0, 0]);
        shared.stop();
        assert!(!shared.push(0, 0, &[1.0]), "an old epoch adds nothing");
    }
}
