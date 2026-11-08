//! Dictation in the pill: the microphone, the speech model and the hand-over of the text.
//!
//! One thread owns the microphone and the model. The window thread sends it three jobs:
//! start, finish and cancel. A finish runs the whole path after the capture step: resample
//! to 16 kHz, transcribe, then give the text to the core with `voice.transcript`. The core
//! applies the vocabulary and types the text into the focused pane. With no focused pane,
//! the pill copies the text with `wl-copy`.
//!
//! Toggle dictation has no key release, so it has no end to wait for. While it listens, the
//! thread takes the audio from the microphone every [`POLL`], resamples it, and gives it to the
//! voice detector's cutter. At each pause of [`PAUSE`] the cutter closes a piece. The thread
//! transcribes the piece and sends it at once. A piece after the first starts with a space.
//! Hold dictation does not use the detector: it keeps the one-clip path.
//!
//! Conversation mode listens for one turn at a time. A turn ends at a pause of
//! [`TURN_PAUSE`]. The thread then closes the microphone, so nothing is heard while the pane
//! works and the voice speaks. It transcribes the turn and reports what became of it with
//! `Input::Turn`: the word "stop" alone ends the mode, a turn with no text sends nothing, and
//! any other text goes to the core, which submits it to the target pane.
//!
//! The microphone, the model and the core are traits, so a test feeds a WAV file through
//! the same path with no microphone.

use crate::link::{self, LinkEvent, LinkSender};
use crate::state::{Effect, Input, TURN_PAUSE, Turn};
use agentsi_voice::stt::{self, Clip, Resampler};
use agentsi_voice::vad::{self, Classifier, Cutter};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The samples that one recording keeps: the pill limit of 120 s, and a margin.
const MAX_RECORDING: Duration = Duration::from_secs(125);
/// The line when the model files are missing.
pub const NOT_DOWNLOADED: &str = "Download the speech-in model in the settings";
/// The line when a key is pressed while the model loads.
pub const LOADING: &str = "The speech-in model is loading. Try again in a few seconds.";
/// The line when no text came out of the recording.
pub const HEARD_NOTHING: &str = "Nothing was heard";
/// The line when toggle dictation starts and the voice detector is missing.
pub const NO_DETECTOR: &str = "Download the voice detector in the settings";
/// How often toggle dictation takes audio from the microphone.
pub const POLL: Duration = Duration::from_millis(50);
/// The pause that ends a piece of toggle dictation, in seconds.
pub const PAUSE: f32 = 0.6;
/// The longest turn of conversation mode. A longer one is cut into two turns.
const TURN_LONGEST: Duration = Duration::from_secs(120);

/// A recording that the owner is making.
pub trait Mic: Send {
    fn start(&mut self) -> Result<(), String>;
    /// Ends the recording and gives what it heard.
    ///
    /// For toggle dictation, it gives what came after the last [`Mic::take`].
    fn finish(&mut self) -> Result<Clip, String>;
    /// Gives what the microphone heard since the start or the last call, and goes on.
    fn take(&mut self) -> Result<Clip, String>;
    /// Ends the recording and drops it.
    fn cancel(&mut self);
}

/// Turns 16 kHz mono samples into text.
pub trait Recognizer: Send {
    fn transcribe(&mut self, samples: &[f32]) -> Result<String, String>;
}

/// Where the text goes: the core, which types it into the focused pane.
pub trait Delivery: Send {
    /// The text after the vocabulary, and the terminal that got it.
    fn deliver(&mut self, text: &str) -> Result<Delivered, String>;
    /// Copies text when no pane has the focus.
    fn copy(&mut self, text: &str) -> Result<(), String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivered {
    pub text: String,
    pub terminal: Option<String>,
}

/// The voice detector of the catalog, or the line that says why it is missing.
pub type OpenDetector = Box<dyn FnMut() -> Result<Box<dyn Classifier + Send>, String> + Send>;
pub type OpenMic = Box<dyn FnMut() -> Result<Box<dyn Mic>, String> + Send>;
/// Called at the start, and again at a key press while the last call failed, so a model that
/// the owner downloads later works without a restart of the pill.
pub type OpenRecognizer = Box<dyn FnMut() -> Result<Box<dyn Recognizer>, String> + Send>;

/// What a recording does with the audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Hold dictation: one clip, transcribed at the release.
    Hold,
    /// Toggle dictation: a piece at each pause of [`PAUSE`], until a stop.
    Toggle,
    /// Conversation mode: the audio up to the first pause of [`TURN_PAUSE`].
    Turn,
}

enum Job {
    Start {
        ticket: u64,
        kind: Kind,
    },
    Finish(u64),
    Cancel,
    /// Loads the model again after a failed load.
    Reload,
}

/// What the model is doing. The window thread reads it at each key press, so a press never
/// waits for a model to load: it opens the microphone only when the model is ready.
enum Slot {
    Loading,
    /// The speech model is ready. `detector` holds the line that says why toggle dictation
    /// cannot run, or `None` when it can.
    Ready {
        detector: Option<String>,
    },
    Failed(String),
}

/// The dictation of the pill. The window thread calls [`Dictator::effect`].
pub struct Dictator {
    jobs: Mutex<mpsc::Sender<Job>>,
    slot: Arc<Mutex<Slot>>,
    events: LinkSender,
}

impl Dictator {
    /// Starts the thread. It loads the models at once, because that takes seconds. A key press
    /// before the speech model is ready fails at once with [`LOADING`], and the microphone stays
    /// closed. Toggle dictation needs the voice detector too, and fails with [`NO_DETECTOR`] when
    /// its file is missing.
    pub fn start(
        open_mic: OpenMic,
        open_recognizer: OpenRecognizer,
        open_detector: OpenDetector,
        delivery: Box<dyn Delivery>,
        events: LinkSender,
        stats: bool,
    ) -> Dictator {
        Dictator::start_polling(
            POLL,
            open_mic,
            open_recognizer,
            open_detector,
            delivery,
            events,
            stats,
        )
    }

    /// [`Dictator::start`] with the time between two takes of toggle dictation. A test sets it
    /// short, so that minutes of audio pass in a moment.
    pub fn start_polling(
        poll: Duration,
        open_mic: OpenMic,
        open_recognizer: OpenRecognizer,
        open_detector: OpenDetector,
        delivery: Box<dyn Delivery>,
        events: LinkSender,
        stats: bool,
    ) -> Dictator {
        let (tx, rx) = mpsc::channel();
        let slot = Arc::new(Mutex::new(Slot::Loading));
        let shared = slot.clone();
        let worker_events = events.clone();
        std::thread::Builder::new()
            .name("dictation".into())
            .spawn(move || {
                Worker {
                    jobs: rx,
                    events: worker_events,
                    poll,
                    open_mic,
                    mic: None,
                    delivery,
                    cutter: None,
                    live: None,
                    stats,
                }
                .run(open_recognizer, open_detector, &shared)
            })
            .expect("the OS starts a thread");
        Dictator {
            jobs: Mutex::new(tx),
            slot,
            events,
        }
    }

    pub fn effect(&self, effect: &Effect) {
        let job = match effect {
            Effect::StartRecording { ticket, toggle } => {
                let kind = if *toggle { Kind::Toggle } else { Kind::Hold };
                match self.admit(*ticket, kind) {
                    Some(job) => job,
                    None => return,
                }
            }
            Effect::Listen { ticket } => match self.admit(*ticket, Kind::Turn) {
                Some(job) => job,
                None => return,
            },
            Effect::FinishRecording { ticket } => Job::Finish(*ticket),
            Effect::CancelRecording => Job::Cancel,
            _ => return,
        };
        self.send(job);
    }

    /// The job for a key press, or `None` when the press is refused. A failed model loads
    /// again, so a model that the owner downloads later works without a restart.
    fn admit(&self, ticket: u64, kind: Kind) -> Option<Job> {
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        let refused = match &*slot {
            Slot::Ready { detector } => match detector {
                Some(why) if kind != Kind::Hold => {
                    let why = why.clone();
                    *slot = Slot::Loading;
                    self.send(Job::Reload);
                    why
                }
                _ => return Some(Job::Start { ticket, kind }),
            },
            Slot::Loading => LOADING.to_owned(),
            Slot::Failed(e) => {
                let e = e.clone();
                *slot = Slot::Loading;
                self.send(Job::Reload);
                e
            }
        };
        let _ = self
            .events
            .unbounded_send(LinkEvent::Input(Input::Dictated {
                ticket,
                result: Err(refused),
            }));
        None
    }

    fn send(&self, job: Job) {
        let _ = self
            .jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(job);
    }

    /// Blocks until the model is not loading, or `limit` passed. For tests.
    pub fn wait_until_loaded(&self, limit: Duration) -> bool {
        let end = Instant::now() + limit;
        loop {
            if !matches!(
                *self.slot.lock().unwrap_or_else(|e| e.into_inner()),
                Slot::Loading
            ) {
                return true;
            }
            if Instant::now() >= end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// Toggle dictation in progress.
struct Live {
    ticket: u64,
    kind: Kind,
    /// Turns the device rate into 16 kHz, with no dip at the edge of a chunk.
    resampler: Option<Resampler>,
    /// Pieces that gave text. The next one starts with a space.
    pieces: usize,
    /// The text of the pieces that no pane took, which the clipboard holds as one text.
    clipboard: String,
    /// The pill got the report on a long quiet already.
    reported: bool,
}

type DetectorCutter = Cutter<Box<dyn Classifier + Send>>;

struct Worker {
    jobs: mpsc::Receiver<Job>,
    events: LinkSender,
    poll: Duration,
    open_mic: OpenMic,
    mic: Option<Box<dyn Mic>>,
    delivery: Box<dyn Delivery>,
    /// The cutter, with the loaded voice detector. `None` when the detector is missing.
    cutter: Option<DetectorCutter>,
    live: Option<Live>,
    stats: bool,
}

/// Samples of 16 kHz audio without speech that end toggle dictation.
fn quiet_limit() -> usize {
    crate::state::QUIET_LIMIT.as_secs() as usize * vad::RATE as usize
}

impl Worker {
    fn send(&self, input: Input) {
        let _ = self.events.unbounded_send(LinkEvent::Input(input));
    }

    fn ended(&self, ticket: u64, result: Result<(), String>) {
        self.send(Input::Dictated { ticket, result });
    }

    fn load(
        &mut self,
        open_recognizer: &mut OpenRecognizer,
        open_detector: &mut OpenDetector,
        slot: &Mutex<Slot>,
    ) -> Option<Box<dyn Recognizer>> {
        let opened = Instant::now();
        let (recognizer, state) = match open_recognizer() {
            Ok(r) => {
                eprintln!("agentsi-pill: speech in ready in {:?}", opened.elapsed());
                // The detector is optional: hold dictation does not use it.
                let detector = match open_detector() {
                    Ok(classifier) => {
                        self.cutter = Some(Cutter::new(classifier, PAUSE));
                        None
                    }
                    Err(e) => {
                        eprintln!("agentsi-pill: voice detector cannot start: {e}");
                        self.cutter = None;
                        Some(e)
                    }
                };
                (Some(r), Slot::Ready { detector })
            }
            Err(e) => {
                eprintln!("agentsi-pill: speech in cannot start: {e}");
                (None, Slot::Failed(e))
            }
        };
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = state;
        recognizer
    }

    fn run(
        mut self,
        mut open_recognizer: OpenRecognizer,
        mut open_detector: OpenDetector,
        slot: &Mutex<Slot>,
    ) {
        let mut recognizer = self.load(&mut open_recognizer, &mut open_detector, slot);
        loop {
            // Toggle dictation takes audio between the jobs. Every other state waits for a job.
            let job = if self.live.is_some() {
                match self.jobs.recv_timeout(self.poll) {
                    Ok(job) => Some(job),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            } else {
                match self.jobs.recv() {
                    Ok(job) => Some(job),
                    Err(_) => return,
                }
            };
            let Some(job) = job else {
                if let Some(recognizer) = recognizer.as_mut() {
                    self.tick(recognizer.as_mut());
                }
                continue;
            };
            match job {
                Job::Start { ticket, kind } => {
                    if let Err(e) = self.start(ticket, kind) {
                        eprintln!("agentsi-pill: cannot record: {e}");
                        self.ended(ticket, Err(e));
                    }
                }
                Job::Cancel => {
                    self.live = None;
                    if let Some(cutter) = self.cutter.as_mut() {
                        cutter.reset();
                    }
                    if let Some(mic) = self.mic.as_mut() {
                        mic.cancel();
                    }
                }
                Job::Reload => {
                    recognizer = self.load(&mut open_recognizer, &mut open_detector, slot)
                }
                Job::Finish(ticket) => {
                    // Without a model no recording started: the press was refused already.
                    let Some(recognizer) = recognizer.as_mut() else {
                        continue;
                    };
                    let running = self.live.as_ref().map(|l| l.ticket);
                    let result = match running {
                        Some(live) if live == ticket => self.finish_toggle(recognizer.as_mut()),
                        // The stop of an old dictation: the one that runs goes on.
                        Some(_) => continue,
                        None => self.finish(recognizer.as_mut()),
                    };
                    match result {
                        Ok(()) => self.ended(ticket, Ok(())),
                        Err(e) => {
                            eprintln!("agentsi-pill: dictation failed: {e}");
                            self.ended(ticket, Err(e));
                        }
                    }
                }
            }
        }
    }

    fn start(&mut self, ticket: u64, kind: Kind) -> Result<(), String> {
        if self.mic.is_none() {
            self.mic = Some((self.open_mic)()?);
        }
        let mic = self.mic.as_mut().expect("set above");
        mic.start().inspect_err(|_| self.mic = None)?;
        if kind != Kind::Hold {
            let cutter = self.cutter.as_mut().ok_or(NO_DETECTOR)?;
            cutter.reset();
            if kind == Kind::Turn {
                cutter.set_pause(TURN_PAUSE);
                cutter.set_longest(TURN_LONGEST);
            } else {
                cutter.set_pause(PAUSE);
                cutter.set_longest(vad::MAX_PIECE);
            }
            self.live = Some(Live {
                ticket,
                kind,
                resampler: None,
                pieces: 0,
                clipboard: String::new(),
                reported: false,
            });
        } else {
            self.live = None;
        }
        Ok(())
    }

    /// One poll of toggle dictation: the audio so far goes through the cutter, and each piece
    /// that closes goes to the core.
    fn tick(&mut self, recognizer: &mut dyn Recognizer) {
        let Some((ticket, kind)) = self.live.as_ref().map(|l| (l.ticket, l.kind)) else {
            return;
        };
        if kind == Kind::Turn {
            return self.tick_turn(recognizer, ticket);
        }
        let result = self.pump(recognizer, false);
        if let Err(e) = result {
            eprintln!("agentsi-pill: dictation failed: {e}");
            self.abort();
            self.ended(ticket, Err(e));
        } else if let (Some(live), Some(cutter)) = (self.live.as_mut(), self.cutter.as_ref())
            && !live.reported
            && cutter.quiet_samples() >= quiet_limit()
        {
            live.reported = true;
            self.send(Input::Quiet { ticket });
        }
    }

    /// One poll of conversation mode. The first piece that closes is the turn.
    fn tick_turn(&mut self, recognizer: &mut dyn Recognizer, ticket: u64) {
        let pieces = match self.collect(false) {
            Ok(pieces) => pieces,
            Err(e) => {
                eprintln!("agentsi-pill: conversation failed: {e}");
                self.abort();
                self.send(Input::Turn {
                    ticket,
                    turn: Turn::Failed(e),
                });
                return;
            }
        };
        if let Some(piece) = pieces.into_iter().next() {
            // The microphone closes at once. Audio that comes while the pane works and the
            // voice speaks is never heard.
            self.abort();
            self.send(Input::Turn {
                ticket,
                turn: Turn::Heard,
            });
            let turn = self.deliver_turn(recognizer, &piece);
            self.send(Input::Turn { ticket, turn });
        } else if let (Some(live), Some(cutter)) = (self.live.as_mut(), self.cutter.as_ref())
            && !live.reported
            && cutter.quiet_samples() >= quiet_limit()
        {
            live.reported = true;
            self.send(Input::Quiet { ticket });
        }
    }

    /// Transcribes a turn and says what became of it.
    fn deliver_turn(&mut self, recognizer: &mut dyn Recognizer, piece: &[f32]) -> Turn {
        let text = match recognizer.transcribe(piece) {
            Ok(text) => text,
            Err(e) => return Turn::Failed(e),
        };
        let text = text.trim();
        if text.is_empty() {
            return Turn::Nothing;
        }
        if is_stop_word(text) {
            return Turn::Stop;
        }
        match self.delivery.deliver(text) {
            Ok(Delivered {
                terminal: Some(_), ..
            }) => Turn::Sent,
            Ok(_) => Turn::Nothing,
            Err(e) => Turn::Failed(e),
        }
    }

    /// Ends the session of toggle dictation and drops the microphone.
    fn abort(&mut self) {
        self.live = None;
        if let Some(cutter) = self.cutter.as_mut() {
            cutter.reset();
        }
        if let Some(mic) = self.mic.as_mut() {
            mic.cancel();
        }
    }

    /// Takes the audio that came in since the last call, or with `last` the rest of it, and
    /// sends the pieces that it closes.
    fn pump(&mut self, recognizer: &mut dyn Recognizer, last: bool) -> Result<(), String> {
        for piece in self.collect(last)? {
            self.send_piece(recognizer, &piece)?;
        }
        Ok(())
    }

    /// The pieces that the audio so far closes. With `last`, the rest of it closes the open one.
    fn collect(&mut self, last: bool) -> Result<Vec<Vec<f32>>, String> {
        let mic = self.mic.as_mut().ok_or("The microphone is not open")?;
        let clip = if last { mic.finish()? } else { mic.take()? };
        let live = self
            .live
            .as_mut()
            .ok_or("Toggle dictation is not running")?;
        let resampler = live
            .resampler
            .get_or_insert_with(|| Resampler::new(clip.rate, stt::RATE));
        let audio = resampler.push(&clip.samples);
        let cutter = self.cutter.as_mut().ok_or(NO_DETECTOR)?;
        let mut pieces = cutter.feed(&audio).map_err(|e| e.to_string())?;
        if last {
            pieces.extend(cutter.finish());
        }
        Ok(pieces)
    }

    /// The end of toggle dictation: the rest of the audio, then the last piece. It fails when
    /// the whole dictation gave no text.
    fn finish_toggle(&mut self, recognizer: &mut dyn Recognizer) -> Result<(), String> {
        let result = self.pump(recognizer, true);
        let pieces = self.live.take().map_or(0, |l| l.pieces);
        if let Some(cutter) = self.cutter.as_mut() {
            cutter.reset();
        }
        result?;
        if pieces == 0 {
            return Err(HEARD_NOTHING.into());
        }
        Ok(())
    }

    /// Transcribes one piece and gives its text to the core. A text that no pane can take
    /// goes to the clipboard, with the text of the earlier pieces, and the dictation goes on.
    fn send_piece(&mut self, recognizer: &mut dyn Recognizer, piece: &[f32]) -> Result<(), String> {
        let text = recognizer.transcribe(piece)?;
        let text = text.trim();
        if text.is_empty() {
            return Ok(());
        }
        let live = self
            .live
            .as_mut()
            .ok_or("Toggle dictation is not running")?;
        let sent = if live.pieces == 0 {
            text.to_owned()
        } else {
            format!(" {text}")
        };
        live.pieces += 1;
        let copy = match self.delivery.deliver(&sent) {
            Ok(Delivered {
                terminal: Some(_), ..
            }) => return Ok(()),
            Ok(d) => d.text,
            Err(e) => {
                eprintln!("agentsi-pill: a piece was not delivered: {e}");
                sent
            }
        };
        live.clipboard.push_str(&copy);
        self.delivery.copy(live.clipboard.trim_start())
    }

    /// Capture is done: the clip goes through the model and the core.
    fn finish(&mut self, recognizer: &mut dyn Recognizer) -> Result<(), String> {
        let released = Instant::now();
        let mic = self.mic.as_mut().ok_or("The microphone is not open")?;
        let clip = mic.finish()?;
        let captured = released.elapsed();
        let samples = clip.at_model_rate();
        let resampled = released.elapsed();
        let text = recognizer.transcribe(&samples)?;
        let transcribed = released.elapsed();
        if text.trim().is_empty() {
            return Err(HEARD_NOTHING.into());
        }
        let delivered = match self.delivery.deliver(&text) {
            Ok(d) => d,
            Err(e) => {
                // The sentence is not lost when the pane cannot take it.
                let _ = self.delivery.copy(&text);
                return Err(e);
            }
        };
        if delivered.terminal.is_none() && !delivered.text.is_empty() {
            self.delivery.copy(&delivered.text)?;
        }
        if self.stats {
            eprintln!(
                "agentsi-pill: dictation of {:.2} s: capture {:?}, resample {:?}, transcribe {:?}, delivered {:?}",
                clip.seconds(),
                captured,
                resampled - captured,
                transcribed - resampled,
                released.elapsed() - transcribed,
            );
        }
        Ok(())
    }
}

/// A turn that is the word "stop" or "para" alone, in any case, with or without punctuation.
pub fn is_stop_word(text: &str) -> bool {
    let mut words = text.split_whitespace();
    let (Some(word), None) = (words.next(), words.next()) else {
        return false;
    };
    let word: String = word
        .trim_matches(|c: char| !c.is_alphanumeric())
        .chars()
        .flat_map(char::to_lowercase)
        .collect();
    word == "stop" || word == "para"
}

/// The model of the catalog, loaded from the standard folders.
pub fn open_recognizer() -> OpenRecognizer {
    Box::new(|| {
        let dir = stt::model_dir().ok_or("HOME is not set")?;
        match stt::Transcriber::open(&dir) {
            Ok(t) => Ok(Box::new(Parakeet(t)) as Box<dyn Recognizer>),
            Err(stt::Error::NotDownloaded(_)) => Err(NOT_DOWNLOADED.to_owned()),
            Err(e) => Err(e.to_string()),
        }
    })
}

/// The voice detector of the catalog, loaded from the standard folders.
pub fn open_detector() -> OpenDetector {
    Box::new(|| {
        let dir = vad::model_dir().ok_or("HOME is not set")?;
        match vad::Silero::open(&dir) {
            Ok(d) => Ok(Box::new(d) as Box<dyn Classifier + Send>),
            Err(vad::Error::NotDownloaded(_)) => Err(NO_DETECTOR.to_owned()),
            Err(e) => Err(e.to_string()),
        }
    })
}

struct Parakeet(stt::Transcriber);

impl Recognizer for Parakeet {
    fn transcribe(&mut self, samples: &[f32]) -> Result<String, String> {
        self.0.transcribe(samples).map_err(|e| e.to_string())
    }
}

/// The local core and the clipboard program.
pub struct CoreDelivery;

impl Delivery for CoreDelivery {
    fn deliver(&mut self, text: &str) -> Result<Delivered, String> {
        link::send_transcript(text).map(|r| Delivered {
            text: r.text,
            terminal: r.terminal,
        })
    }

    fn copy(&mut self, text: &str) -> Result<(), String> {
        let missing = |e: std::io::Error| format!("No pane has the focus, and wl-copy failed: {e}");
        let mut child = Command::new("wl-copy")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(missing)?;
        child
            .stdin
            .take()
            .ok_or("wl-copy has no input")?
            .write_all(text.as_bytes())
            .map_err(missing)?;
        // wl-copy stays alive to serve the selection, so the pill does not wait for it.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

/// The microphone through `cpal`. The stream opens at each start and closes at each finish,
/// so the device is free between two dictations.
pub struct CpalMic {
    wanted: Option<String>,
    capture: Option<Capture>,
}

struct Capture {
    stream: cpal::Stream,
    rate: u32,
    buffer: Arc<Mutex<Vec<f32>>>,
    failed: Arc<Mutex<Option<String>>>,
}

impl CpalMic {
    pub fn open(wanted: Option<String>) -> OpenMic {
        Box::new(move || {
            Ok(Box::new(CpalMic {
                wanted: wanted.clone(),
                capture: None,
            }) as Box<dyn Mic>)
        })
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buffer: Arc<Mutex<Vec<f32>>>,
    failed: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    let cap = (config.sample_rate as f32 * MAX_RECORDING.as_secs_f32()) as usize;
    device
        .build_input_stream(
            *config,
            move |data: &[T], _| {
                let mut buffer = buffer.lock().unwrap_or_else(|e| e.into_inner());
                let room = cap.saturating_sub(buffer.len());
                buffer.extend(data.chunks_exact(channels).take(room).map(|f| {
                    f.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / channels as f32
                }));
            },
            move |e| {
                eprintln!("agentsi-pill: the input stream failed: {e}");
                *failed.lock().unwrap_or_else(|p| p.into_inner()) = Some(e.to_string());
            },
            None,
        )
        .map_err(|e| e.to_string())
}

impl Mic for CpalMic {
    fn start(&mut self) -> Result<(), String> {
        self.capture = None;
        let device = crate::audio::input_device(self.wanted.as_deref())?;
        let supported = device.default_input_config().map_err(|e| e.to_string())?;
        let config = supported.config();
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let failed = Arc::new(Mutex::new(None));
        let (b, f) = (buffer.clone(), failed.clone());
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, &config, b, f),
            SampleFormat::I16 => build::<i16>(&device, &config, b, f),
            SampleFormat::I32 => build::<i32>(&device, &config, b, f),
            SampleFormat::U16 => build::<u16>(&device, &config, b, f),
            SampleFormat::F64 => build::<f64>(&device, &config, b, f),
            SampleFormat::U8 => build::<u8>(&device, &config, b, f),
            SampleFormat::I8 => build::<i8>(&device, &config, b, f),
            other => Err(format!("the input device gives samples of type {other}")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        self.capture = Some(Capture {
            stream,
            rate: config.sample_rate,
            buffer,
            failed,
        });
        Ok(())
    }

    fn finish(&mut self) -> Result<Clip, String> {
        let capture = self
            .capture
            .take()
            .ok_or("The microphone was not recording")?;
        drop(capture.stream);
        if let Some(e) = capture
            .failed
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            return Err(format!("The microphone stopped: {e}"));
        }
        let samples =
            std::mem::take(&mut *capture.buffer.lock().unwrap_or_else(|e| e.into_inner()));
        Ok(Clip {
            samples,
            rate: capture.rate,
        })
    }

    fn take(&mut self) -> Result<Clip, String> {
        let capture = self
            .capture
            .as_ref()
            .ok_or("The microphone was not recording")?;
        if let Some(e) = capture
            .failed
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            return Err(format!("The microphone stopped: {e}"));
        }
        let samples =
            std::mem::take(&mut *capture.buffer.lock().unwrap_or_else(|e| e.into_inner()));
        Ok(Clip {
            samples,
            rate: capture.rate,
        })
    }

    fn cancel(&mut self) {
        self.capture = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::channel::mpsc::unbounded;

    struct FakeMic {
        clip: Option<Clip>,
        starts: Arc<Mutex<u32>>,
    }

    impl Mic for FakeMic {
        fn start(&mut self) -> Result<(), String> {
            *self.starts.lock().unwrap() += 1;
            Ok(())
        }
        fn finish(&mut self) -> Result<Clip, String> {
            self.clip.take().ok_or_else(|| "no clip".into())
        }
        fn take(&mut self) -> Result<Clip, String> {
            Err("not a streaming microphone".into())
        }
        fn cancel(&mut self) {}
    }

    fn no_detector() -> OpenDetector {
        Box::new(|| Err(NO_DETECTOR.into()))
    }

    struct Words(&'static str);

    impl Recognizer for Words {
        fn transcribe(&mut self, samples: &[f32]) -> Result<String, String> {
            assert!(!samples.is_empty());
            Ok(self.0.into())
        }
    }

    #[derive(Default)]
    struct Log {
        delivered: Vec<String>,
        copied: Vec<String>,
    }

    struct Sink {
        log: Arc<Mutex<Log>>,
        terminal: Option<String>,
    }

    impl Delivery for Sink {
        fn deliver(&mut self, text: &str) -> Result<Delivered, String> {
            self.log.lock().unwrap().delivered.push(text.into());
            Ok(Delivered {
                text: text.into(),
                terminal: self.terminal.clone(),
            })
        }
        fn copy(&mut self, text: &str) -> Result<(), String> {
            self.log.lock().unwrap().copied.push(text.into());
            Ok(())
        }
    }

    struct Rig {
        dictator: Dictator,
        rx: futures::channel::mpsc::UnboundedReceiver<LinkEvent>,
        log: Arc<Mutex<Log>>,
        starts: Arc<Mutex<u32>>,
    }

    fn rig(words: &'static str, terminal: Option<&str>) -> Rig {
        let (tx, rx) = unbounded();
        let log = Arc::new(Mutex::new(Log::default()));
        let starts = Arc::new(Mutex::new(0));
        let s = starts.clone();
        let dictator = Dictator::start(
            Box::new(move || {
                Ok(Box::new(FakeMic {
                    clip: Some(Clip {
                        samples: vec![0.1; 48_000],
                        rate: 48_000,
                    }),
                    starts: s.clone(),
                }) as Box<dyn Mic>)
            }),
            Box::new(move || Ok(Box::new(Words(words)) as Box<dyn Recognizer>)),
            no_detector(),
            Box::new(Sink {
                log: log.clone(),
                terminal: terminal.map(str::to_owned),
            }),
            tx,
            false,
        );
        assert!(dictator.wait_until_loaded(Duration::from_secs(10)));
        Rig {
            dictator,
            rx,
            log,
            starts,
        }
    }

    fn done(ticket: u64) -> Input {
        Input::Dictated {
            ticket,
            result: Ok(()),
        }
    }

    fn failed(ticket: u64, message: impl Into<String>) -> Input {
        Input::Dictated {
            ticket,
            result: Err(message.into()),
        }
    }

    fn next(rx: &mut futures::channel::mpsc::UnboundedReceiver<LinkEvent>) -> Input {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match rx.try_recv() {
                Ok(LinkEvent::Input(input)) => return input,
                Ok(other) => panic!("unexpected {other:?}"),
                Err(_) => {
                    assert!(Instant::now() < deadline, "no event within 10 seconds");
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    #[test]
    fn a_finished_recording_goes_to_the_core_and_hides_the_pill() {
        let Rig {
            dictator,
            mut rx,
            log,
            starts,
        } = rig("Hello world.", Some("t1"));
        dictator.effect(&Effect::StartRecording {
            ticket: 1,
            toggle: false,
        });
        dictator.effect(&Effect::FinishRecording { ticket: 1 });
        assert_eq!(next(&mut rx), done(1));
        assert_eq!(*starts.lock().unwrap(), 1);
        let log = log.lock().unwrap();
        assert_eq!(log.delivered, ["Hello world."]);
        assert!(log.copied.is_empty());
    }

    #[test]
    fn with_no_focused_pane_the_text_is_copied() {
        let Rig {
            dictator,
            mut rx,
            log,
            ..
        } = rig("Copy me.", None);
        dictator.effect(&Effect::StartRecording {
            ticket: 1,
            toggle: false,
        });
        dictator.effect(&Effect::FinishRecording { ticket: 1 });
        assert_eq!(next(&mut rx), done(1));
        assert_eq!(log.lock().unwrap().copied, ["Copy me."]);
    }

    struct Refusing(Arc<Mutex<Log>>);

    impl Delivery for Refusing {
        fn deliver(&mut self, _: &str) -> Result<Delivered, String> {
            Err("The pane closed".into())
        }
        fn copy(&mut self, text: &str) -> Result<(), String> {
            self.0.lock().unwrap().copied.push(text.into());
            Ok(())
        }
    }

    #[test]
    fn a_sentence_that_the_core_cannot_deliver_is_copied() {
        let (tx, mut rx) = unbounded();
        let log = Arc::new(Mutex::new(Log::default()));
        let dictator = Dictator::start(
            Box::new(|| {
                Ok(Box::new(FakeMic {
                    clip: Some(Clip {
                        samples: vec![0.1; 48_000],
                        rate: 48_000,
                    }),
                    starts: Arc::default(),
                }) as Box<dyn Mic>)
            }),
            Box::new(|| Ok(Box::new(Words("Keep me.")) as Box<dyn Recognizer>)),
            no_detector(),
            Box::new(Refusing(log.clone())),
            tx,
            false,
        );
        assert!(dictator.wait_until_loaded(Duration::from_secs(10)));
        dictator.effect(&Effect::StartRecording {
            ticket: 1,
            toggle: false,
        });
        dictator.effect(&Effect::FinishRecording { ticket: 1 });
        assert_eq!(next(&mut rx), failed(1, "The pane closed"));
        assert_eq!(log.lock().unwrap().copied, ["Keep me."]);
    }

    #[test]
    fn a_silent_recording_shows_an_error_and_types_nothing() {
        let Rig {
            dictator,
            mut rx,
            log,
            ..
        } = rig("  ", Some("t1"));
        dictator.effect(&Effect::StartRecording {
            ticket: 1,
            toggle: false,
        });
        dictator.effect(&Effect::FinishRecording { ticket: 1 });
        assert_eq!(next(&mut rx), failed(1, HEARD_NOTHING));
        assert!(log.lock().unwrap().delivered.is_empty());
    }

    #[test]
    fn a_model_that_cannot_load_fails_at_the_key_press() {
        let (tx, mut rx) = unbounded();
        let dictator = Dictator::start(
            Box::new(|| Err("no microphone".into())),
            Box::new(|| Err(NOT_DOWNLOADED.into())),
            no_detector(),
            Box::new(Sink {
                log: Arc::default(),
                terminal: None,
            }),
            tx,
            false,
        );
        assert!(dictator.wait_until_loaded(Duration::from_secs(10)));
        dictator.effect(&Effect::StartRecording {
            ticket: 1,
            toggle: false,
        });
        assert_eq!(next(&mut rx), failed(1, NOT_DOWNLOADED));
    }

    #[test]
    fn a_key_press_while_the_model_loads_is_refused_and_opens_no_microphone() {
        let (tx, mut rx) = unbounded();
        let (release, gate) = mpsc::channel::<()>();
        let starts = Arc::new(Mutex::new(0));
        let s = starts.clone();
        let dictator = Dictator::start(
            Box::new(move || {
                Ok(Box::new(FakeMic {
                    clip: Some(Clip {
                        samples: vec![0.1; 48_000],
                        rate: 48_000,
                    }),
                    starts: s.clone(),
                }) as Box<dyn Mic>)
            }),
            Box::new(move || {
                gate.recv().unwrap();
                Ok(Box::new(Words("Hello.")) as Box<dyn Recognizer>)
            }),
            no_detector(),
            Box::new(Sink {
                log: Arc::default(),
                terminal: Some("t1".into()),
            }),
            tx,
            false,
        );
        // The model is still loading: the pill says so at once and the microphone stays closed.
        dictator.effect(&Effect::StartRecording {
            ticket: 1,
            toggle: false,
        });
        assert_eq!(next(&mut rx), failed(1, LOADING));
        assert_eq!(*starts.lock().unwrap(), 0);
        release.send(()).unwrap();
        assert!(dictator.wait_until_loaded(Duration::from_secs(10)));
        dictator.effect(&Effect::StartRecording {
            ticket: 2,
            toggle: false,
        });
        dictator.effect(&Effect::FinishRecording { ticket: 2 });
        assert_eq!(next(&mut rx), done(2));
        assert_eq!(*starts.lock().unwrap(), 1);
    }

    #[test]
    fn a_model_downloaded_after_the_pill_started_loads_at_the_next_key_press() {
        let (tx, mut rx) = unbounded();
        let opens = Arc::new(Mutex::new(0));
        let n = opens.clone();
        let dictator = Dictator::start(
            Box::new(|| {
                Ok(Box::new(FakeMic {
                    clip: Some(Clip {
                        samples: vec![0.1; 48_000],
                        rate: 48_000,
                    }),
                    starts: Arc::default(),
                }) as Box<dyn Mic>)
            }),
            Box::new(move || {
                let mut n = n.lock().unwrap();
                *n += 1;
                if *n == 1 {
                    Err(NOT_DOWNLOADED.into())
                } else {
                    Ok(Box::new(Words("Hello.")) as Box<dyn Recognizer>)
                }
            }),
            no_detector(),
            Box::new(Sink {
                log: Arc::default(),
                terminal: Some("t1".into()),
            }),
            tx,
            false,
        );
        assert!(dictator.wait_until_loaded(Duration::from_secs(10)));
        dictator.effect(&Effect::StartRecording {
            ticket: 1,
            toggle: false,
        });
        assert_eq!(next(&mut rx), failed(1, NOT_DOWNLOADED));
        // The press started a new load, which now finds the files.
        assert!(dictator.wait_until_loaded(Duration::from_secs(10)));
        dictator.effect(&Effect::StartRecording {
            ticket: 2,
            toggle: false,
        });
        dictator.effect(&Effect::FinishRecording { ticket: 2 });
        assert_eq!(next(&mut rx), done(2));
        assert_eq!(*opens.lock().unwrap(), 2);
    }

    #[test]
    fn a_microphone_that_will_not_open_fails_the_start() {
        let (tx, mut rx) = unbounded();
        let dictator = Dictator::start(
            Box::new(|| Err("No audio input device".into())),
            Box::new(|| Ok(Box::new(Words("x")) as Box<dyn Recognizer>)),
            no_detector(),
            Box::new(Sink {
                log: Arc::default(),
                terminal: None,
            }),
            tx,
            false,
        );
        assert!(dictator.wait_until_loaded(Duration::from_secs(10)));
        dictator.effect(&Effect::StartRecording {
            ticket: 1,
            toggle: false,
        });
        assert_eq!(next(&mut rx), failed(1, "No audio input device"));
    }

    /// A microphone that plays a script of 16 kHz samples, `chunk` samples at each take.
    struct ScriptMic {
        audio: Vec<f32>,
        at: usize,
        chunk: usize,
        /// Set when a take finds the script over.
        over: Arc<std::sync::atomic::AtomicBool>,
        /// Samples that the takes gave so far.
        given: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Mic for ScriptMic {
        fn start(&mut self) -> Result<(), String> {
            self.at = 0;
            Ok(())
        }
        fn take(&mut self) -> Result<Clip, String> {
            let end = (self.at + self.chunk).min(self.audio.len());
            let samples = self.audio[self.at..end].to_vec();
            if samples.is_empty() {
                self.over.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            self.at = end;
            self.given.store(end, std::sync::atomic::Ordering::SeqCst);
            Ok(Clip {
                samples,
                rate: 16_000,
            })
        }
        fn finish(&mut self) -> Result<Clip, String> {
            let samples = self.audio[self.at..].to_vec();
            self.at = self.audio.len();
            Ok(Clip {
                samples,
                rate: 16_000,
            })
        }
        fn cancel(&mut self) {}
    }

    /// Speech where a frame holds a sample above 0.1.
    struct Loud;

    impl Classifier for Loud {
        fn speech(&mut self, frame: &[f32]) -> Result<f32, vad::Error> {
            Ok(if frame.iter().any(|s| s.abs() > 0.1) {
                0.9
            } else {
                0.0
            })
        }
        fn reset(&mut self) {}
    }

    fn loud() -> OpenDetector {
        Box::new(|| Ok(Box::new(Loud) as Box<dyn Classifier + Send>))
    }

    /// Names each piece "p1", "p2", and keeps the length of each piece in samples.
    struct Counting(Arc<Mutex<Vec<usize>>>);

    impl Recognizer for Counting {
        fn transcribe(&mut self, samples: &[f32]) -> Result<String, String> {
            let mut lengths = self.0.lock().unwrap();
            lengths.push(samples.len());
            Ok(format!("p{}", lengths.len()))
        }
    }

    fn speech(seconds: f32) -> Vec<f32> {
        (0..(seconds * 16_000.0) as usize)
            .map(|i| if i % 2 == 0 { 0.5 } else { -0.5 })
            .collect()
    }

    fn quiet(seconds: f32) -> Vec<f32> {
        vec![0.0; (seconds * 16_000.0) as usize]
    }

    /// A pill with a scripted microphone, ready for toggle dictation.
    struct Toggle {
        dictator: Dictator,
        rx: futures::channel::mpsc::UnboundedReceiver<LinkEvent>,
        log: Arc<Mutex<Log>>,
        lengths: Arc<Mutex<Vec<usize>>>,
        /// The microphone played its script to the end.
        over: Arc<std::sync::atomic::AtomicBool>,
        /// Samples that the microphone gave so far.
        given: Arc<std::sync::atomic::AtomicUsize>,
    }

    fn toggle_rig(audio: Vec<f32>, terminal: Option<&str>, detector: OpenDetector) -> Toggle {
        let (tx, rx) = unbounded();
        let log = Arc::new(Mutex::new(Log::default()));
        let lengths = Arc::new(Mutex::new(Vec::new()));
        let counted = lengths.clone();
        let over = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let script_over = over.clone();
        let given = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let script_given = given.clone();
        let dictator = Dictator::start_polling(
            Duration::from_millis(1),
            Box::new(move || {
                Ok(Box::new(ScriptMic {
                    audio: audio.clone(),
                    at: 0,
                    chunk: 8_000,
                    over: script_over.clone(),
                    given: script_given.clone(),
                }) as Box<dyn Mic>)
            }),
            Box::new(move || Ok(Box::new(Counting(counted.clone())) as Box<dyn Recognizer>)),
            detector,
            Box::new(Sink {
                log: log.clone(),
                terminal: terminal.map(str::to_owned),
            }),
            tx,
            false,
        );
        assert!(dictator.wait_until_loaded(Duration::from_secs(10)));
        Toggle {
            dictator,
            rx,
            log,
            lengths,
            over,
            given,
        }
    }

    fn start_toggle(ticket: u64) -> Effect {
        Effect::StartRecording {
            ticket,
            toggle: true,
        }
    }

    fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !ok() {
            assert!(Instant::now() < deadline, "timed out: {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn toggle_dictation_sends_the_text_in_pieces_during_3_minutes_of_speech() {
        // Phrases of 7 to 40 s with a pause of 0.9 s. The long ones pass the limit of a piece.
        let mut audio = Vec::new();
        let mut seconds = 0.0;
        let mut n = 0;
        while seconds < 180.0 {
            let length = 7.0 + (n * 13 % 34) as f32;
            audio.extend(speech(length));
            audio.extend(quiet(0.9));
            seconds += length + 0.9;
            n += 1;
        }
        let Toggle {
            dictator,
            mut rx,
            log,
            lengths,
            over,
            ..
        } = toggle_rig(audio, Some("t1"), loud());
        dictator.effect(&start_toggle(1));
        // The text arrives before any stop: wait for the script to end and for the last piece,
        // which the last pause closes.
        wait_for("the script ends", || {
            over.load(std::sync::atomic::Ordering::SeqCst)
        });
        wait_for("the pieces", || {
            let pieces = lengths.lock().unwrap().len();
            log.lock().unwrap().delivered.len() == pieces
        });
        let delivered = log.lock().unwrap().delivered.clone();
        let count = delivered.len();
        assert!(count >= 6, "{count}");
        assert_eq!(delivered[0], "p1");
        for (i, text) in delivered.iter().enumerate().skip(1) {
            assert_eq!(text, &format!(" p{}", i + 1), "in order, joined by a space");
        }
        let lengths = lengths.lock().unwrap().clone();
        assert!(lengths.iter().all(|l| *l <= 30 * 16_000), "{lengths:?}");
        // No stop yet: the dictation has no limit. The stop ends it with no more text.
        assert!(rx.try_recv().is_err());
        dictator.effect(&Effect::FinishRecording { ticket: 1 });
        assert_eq!(next(&mut rx), done(1));
        assert_eq!(log.lock().unwrap().delivered.len(), count);
        assert!(log.lock().unwrap().copied.is_empty());
    }

    #[test]
    fn with_no_focused_pane_the_clipboard_holds_all_the_pieces() {
        let audio = [
            speech(2.0),
            quiet(1.0),
            speech(2.0),
            quiet(1.0),
            speech(2.0),
        ]
        .concat();
        let Toggle {
            dictator,
            mut rx,
            log,
            ..
        } = toggle_rig(audio, None, loud());
        dictator.effect(&start_toggle(1));
        wait_for("two pieces", || log.lock().unwrap().delivered.len() == 2);
        dictator.effect(&Effect::FinishRecording { ticket: 1 });
        assert_eq!(next(&mut rx), done(1));
        let log = log.lock().unwrap();
        assert_eq!(log.delivered, ["p1", " p2", " p3"]);
        // Each copy replaces the clipboard, so each one holds the text so far, with no lead.
        assert_eq!(log.copied, ["p1", "p1 p2", "p1 p2 p3"]);
    }

    #[test]
    fn ten_minutes_with_no_speech_are_reported_once() {
        let audio = [speech(1.0), quiet(10.0 * 60.0 + 2.0)].concat();
        let Toggle {
            dictator, mut rx, ..
        } = toggle_rig(audio, Some("t1"), loud());
        dictator.effect(&start_toggle(7));
        assert_eq!(next(&mut rx), Input::Quiet { ticket: 7 });
        // The cancel that follows ends the session, and no second report comes.
        dictator.effect(&Effect::CancelRecording);
        std::thread::sleep(Duration::from_millis(100));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn toggle_dictation_without_the_voice_detector_is_refused_and_hold_dictation_still_works() {
        let Toggle {
            dictator,
            mut rx,
            log,
            ..
        } = toggle_rig(speech(1.0), Some("t1"), no_detector());
        dictator.effect(&start_toggle(1));
        assert_eq!(next(&mut rx), failed(1, NO_DETECTOR));
        assert!(log.lock().unwrap().delivered.is_empty());
        // The refusal started a new load, which finds no file either.
        assert!(dictator.wait_until_loaded(Duration::from_secs(10)));
        dictator.effect(&Effect::StartRecording {
            ticket: 2,
            toggle: false,
        });
        dictator.effect(&Effect::FinishRecording { ticket: 2 });
        assert_eq!(next(&mut rx), done(2));
        assert_eq!(log.lock().unwrap().delivered, ["p1"]);
    }

    #[test]
    fn toggle_dictation_with_no_speech_at_all_says_nothing_was_heard() {
        let Toggle {
            dictator,
            mut rx,
            log,
            ..
        } = toggle_rig(quiet(2.0), Some("t1"), loud());
        dictator.effect(&start_toggle(1));
        dictator.effect(&Effect::FinishRecording { ticket: 1 });
        assert_eq!(next(&mut rx), failed(1, HEARD_NOTHING));
        assert!(log.lock().unwrap().delivered.is_empty());
    }

    #[test]
    fn a_stop_gives_the_open_piece_and_a_cancel_gives_nothing() {
        let audio = speech(3.0);
        let Toggle {
            dictator,
            mut rx,
            log,
            ..
        } = toggle_rig(audio.clone(), Some("t1"), loud());
        dictator.effect(&start_toggle(1));
        dictator.effect(&Effect::FinishRecording { ticket: 1 });
        assert_eq!(next(&mut rx), done(1));
        assert_eq!(log.lock().unwrap().delivered, ["p1"]);
        // A second dictation, cancelled: its piece is dropped.
        dictator.effect(&start_toggle(2));
        dictator.effect(&Effect::CancelRecording);
        dictator.effect(&start_toggle(3));
        dictator.effect(&Effect::FinishRecording { ticket: 3 });
        assert_eq!(next(&mut rx), done(3));
        assert_eq!(log.lock().unwrap().delivered, ["p1", "p2"]);
    }

    fn heard(ticket: u64) -> Input {
        Input::Turn {
            ticket,
            turn: Turn::Heard,
        }
    }

    fn outcome(ticket: u64, turn: Turn) -> Input {
        Input::Turn { ticket, turn }
    }

    #[test]
    fn a_turn_ends_at_a_pause_of_1_2_s_goes_to_the_core_and_closes_the_microphone() {
        // A pause of 0.9 s inside the turn does not end it. The pause of 1.5 s after it does.
        // The 20 s of quiet after it must stay unheard: the pane works and the voice speaks.
        let audio = [
            speech(1.0),
            quiet(0.9),
            speech(1.0),
            quiet(1.5),
            quiet(20.0),
        ]
        .concat();
        let total = audio.len();
        let Toggle {
            dictator,
            mut rx,
            log,
            lengths,
            given,
            ..
        } = toggle_rig(audio, Some("t1"), loud());
        dictator.effect(&Effect::Listen { ticket: 4 });
        assert_eq!(next(&mut rx), heard(4));
        assert_eq!(next(&mut rx), outcome(4, Turn::Sent));
        // One turn: the two speeches and the short pause between them, as one text.
        assert_eq!(log.lock().unwrap().delivered, ["p1"]);
        let seconds = lengths.lock().unwrap()[0] as f32 / 16_000.0;
        assert!((2.9..3.3).contains(&seconds), "{seconds}");
        // The microphone is closed: it gives no more audio, and no second turn comes.
        let after = given.load(std::sync::atomic::Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(given.load(std::sync::atomic::Ordering::SeqCst), after);
        assert!(after < total, "{after} of {total}");
        assert!(rx.try_recv().is_err());
        // The pill listens again: a new start opens the microphone for the next turn.
        dictator.effect(&Effect::Listen { ticket: 5 });
        assert_eq!(next(&mut rx), heard(5));
        assert_eq!(next(&mut rx), outcome(5, Turn::Sent));
        assert_eq!(log.lock().unwrap().delivered, ["p1", "p2"]);
    }

    #[test]
    fn a_turn_that_gives_no_text_sends_nothing() {
        let audio = [speech(1.0), quiet(2.0)].concat();
        let (tx, mut rx) = unbounded();
        let log = Arc::new(Mutex::new(Log::default()));
        let dictator = Dictator::start_polling(
            Duration::from_millis(1),
            Box::new(move || {
                Ok(Box::new(ScriptMic {
                    audio: audio.clone(),
                    at: 0,
                    chunk: 8_000,
                    over: Arc::default(),
                    given: Arc::default(),
                }) as Box<dyn Mic>)
            }),
            Box::new(|| Ok(Box::new(Words("  ")) as Box<dyn Recognizer>)),
            loud(),
            Box::new(Sink {
                log: log.clone(),
                terminal: Some("t1".into()),
            }),
            tx,
            false,
        );
        assert!(dictator.wait_until_loaded(Duration::from_secs(10)));
        dictator.effect(&Effect::Listen { ticket: 1 });
        assert_eq!(next(&mut rx), heard(1));
        assert_eq!(next(&mut rx), outcome(1, Turn::Nothing));
        assert!(log.lock().unwrap().delivered.is_empty());
    }

    #[test]
    fn the_word_stop_alone_is_not_sent_and_ends_the_mode() {
        for said in ["Stop.", "stop", "  Para! ", "¡Para!"] {
            let audio = [speech(1.0), quiet(2.0)].concat();
            let (tx, mut rx) = unbounded();
            let log = Arc::new(Mutex::new(Log::default()));
            let dictator = Dictator::start_polling(
                Duration::from_millis(1),
                Box::new(move || {
                    Ok(Box::new(ScriptMic {
                        audio: audio.clone(),
                        at: 0,
                        chunk: 8_000,
                        over: Arc::default(),
                        given: Arc::default(),
                    }) as Box<dyn Mic>)
                }),
                Box::new(move || Ok(Box::new(Words(said)) as Box<dyn Recognizer>)),
                loud(),
                Box::new(Sink {
                    log: log.clone(),
                    terminal: Some("t1".into()),
                }),
                tx,
                false,
            );
            assert!(dictator.wait_until_loaded(Duration::from_secs(10)));
            dictator.effect(&Effect::Listen { ticket: 1 });
            assert_eq!(next(&mut rx), heard(1), "{said}");
            assert_eq!(next(&mut rx), outcome(1, Turn::Stop), "{said}");
            assert!(log.lock().unwrap().delivered.is_empty(), "{said}");
        }
        assert!(is_stop_word("STOP"));
        for text in [
            "stop it",
            "please stop",
            "stops",
            "stop stop",
            "",
            "para que",
        ] {
            assert!(!is_stop_word(text), "{text}");
        }
    }

    #[test]
    fn a_turn_with_no_pane_to_take_it_or_a_refusal_is_not_a_sent_turn() {
        let Toggle {
            dictator, mut rx, ..
        } = toggle_rig([speech(1.0), quiet(2.0)].concat(), None, loud());
        dictator.effect(&Effect::Listen { ticket: 1 });
        assert_eq!(next(&mut rx), heard(1));
        assert_eq!(next(&mut rx), outcome(1, Turn::Nothing));
    }

    #[test]
    fn conversation_mode_without_the_voice_detector_is_refused() {
        let Toggle {
            dictator, mut rx, ..
        } = toggle_rig(speech(1.0), Some("t1"), no_detector());
        dictator.effect(&Effect::Listen { ticket: 3 });
        assert_eq!(next(&mut rx), failed(3, NO_DETECTOR));
    }

    #[test]
    fn ten_minutes_with_no_turn_are_reported_once() {
        let audio = [quiet(10.0 * 60.0 + 2.0)].concat();
        let Toggle {
            dictator, mut rx, ..
        } = toggle_rig(audio, Some("t1"), loud());
        dictator.effect(&Effect::Listen { ticket: 7 });
        assert_eq!(next(&mut rx), Input::Quiet { ticket: 7 });
        dictator.effect(&Effect::CancelRecording);
        std::thread::sleep(Duration::from_millis(100));
        assert!(rx.try_recv().is_err());
    }
}
