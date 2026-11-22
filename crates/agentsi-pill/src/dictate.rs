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
//! The speech model and the voice detector load at the first start of a recording, after the
//! microphone opened. The load runs on its own thread, and the thread of the microphone goes on
//! with its jobs: the microphone records while the models load, and a release or a cancel closes
//! it at once. So no audio before the release is lost, and none after it is heard: only the text
//! waits. They drop together when the keep time passes with no recording.
//!
//! The microphone, the model and the core are traits, so a test feeds a WAV file through
//! the same path with no microphone.

use crate::keep::{Clock, KeepTime, Keeper, Policy, dropped_line};
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
/// Called at the first key press, and again at a key press after a failed load or a drop, so a
/// model that the owner downloads later works without a restart of the pill.
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
    Keep(KeepTime),
    /// A load of the speech model ended.
    Loaded(Opened),
    /// The dictator is gone.
    Quit,
}

/// The dictation of the pill. The window thread calls [`Dictator::effect`].
pub struct Dictator {
    jobs: Mutex<mpsc::Sender<Job>>,
}

impl Dictator {
    /// Starts the thread. It loads no model until the first key press, unless the keep time
    /// is "Load at start". A press opens the microphone at once, and the models load while it
    /// records. Toggle dictation needs the voice detector too, and fails with [`NO_DETECTOR`]
    /// when its file is missing.
    pub fn start(
        open_mic: OpenMic,
        open_recognizer: OpenRecognizer,
        open_detector: OpenDetector,
        delivery: Box<dyn Delivery>,
        events: LinkSender,
        policy: Policy,
        stats: bool,
    ) -> Dictator {
        Dictator::start_polling(
            POLL,
            open_mic,
            open_recognizer,
            open_detector,
            delivery,
            events,
            policy,
            stats,
        )
    }

    /// [`Dictator::start`] with the time between two takes of toggle dictation. A test sets it
    /// short, so that minutes of audio pass in a moment.
    #[allow(clippy::too_many_arguments)]
    pub fn start_polling(
        poll: Duration,
        open_mic: OpenMic,
        open_recognizer: OpenRecognizer,
        open_detector: OpenDetector,
        delivery: Box<dyn Delivery>,
        events: LinkSender,
        policy: Policy,
        stats: bool,
    ) -> Dictator {
        let (tx, rx) = mpsc::channel();
        let me = tx.clone();
        std::thread::Builder::new()
            .name("dictation".into())
            .spawn(move || {
                Worker {
                    jobs: rx,
                    me,
                    events,
                    poll,
                    open_mic,
                    mic: None,
                    recording: false,
                    open_recognizer: Arc::new(Mutex::new(open_recognizer)),
                    loading: None,
                    shown: false,
                    waiting: None,
                    open_detector,
                    delivery,
                    cutter: None,
                    live: None,
                    keeper: Keeper::new(policy.keep),
                    clock: policy.clock,
                    stats,
                }
                .run()
            })
            .expect("the OS starts a thread");
        Dictator {
            jobs: Mutex::new(tx),
        }
    }

    pub fn effect(&self, effect: &Effect) {
        let job = match effect {
            Effect::StartRecording { ticket, toggle } => Job::Start {
                ticket: *ticket,
                kind: if *toggle { Kind::Toggle } else { Kind::Hold },
            },
            Effect::Listen { ticket } => Job::Start {
                ticket: *ticket,
                kind: Kind::Turn,
            },
            Effect::FinishRecording { ticket } => Job::Finish(*ticket),
            Effect::CancelRecording => Job::Cancel,
            _ => return,
        };
        self.send(job);
    }

    /// A new keep time for the speech model and the voice detector. It applies at once.
    pub fn set_keep(&self, keep: KeepTime) {
        self.send(Job::Keep(keep));
    }

    fn send(&self, job: Job) {
        let _ = self
            .jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(job);
    }
}

impl Drop for Dictator {
    /// The thread holds a sender of its own jobs for the loads, so it learns the end here.
    fn drop(&mut self) {
        self.send(Job::Quit);
    }
}

/// Why a load of the speech model runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Load {
    /// The keep time "Load at start", with no recording.
    Preload,
    /// A recording waits for it.
    Press,
}

/// A recording that started before the speech model was ready. Its text waits for the model.
struct Waiting {
    ticket: u64,
    kind: Kind,
    /// What the microphone heard up to the release, once the owner released.
    clip: Option<Clip>,
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
    /// For the thread of a load, which sends its result as [`Job::Loaded`].
    me: mpsc::Sender<Job>,
    events: LinkSender,
    poll: Duration,
    open_mic: OpenMic,
    mic: Option<Box<dyn Mic>>,
    /// The microphone records for a dictation or a turn, so the models are in use.
    recording: bool,
    /// Shared with the thread of a load.
    open_recognizer: Arc<Mutex<OpenRecognizer>>,
    /// The load that runs on its own thread. At most one runs.
    loading: Option<Load>,
    /// The pill got `Input::Loading(true)` for the load that runs.
    shown: bool,
    waiting: Option<Waiting>,
    open_detector: OpenDetector,
    delivery: Box<dyn Delivery>,
    /// The cutter, with the loaded voice detector. `None` when the detector is missing or
    /// dropped.
    cutter: Option<DetectorCutter>,
    live: Option<Live>,
    keeper: Keeper,
    clock: Clock,
    stats: bool,
}

type Loaded = Option<Box<dyn Recognizer>>;
/// What a load of the speech model gave, and how long it took.
type Opened = (Result<Box<dyn Recognizer>, String>, Duration);

fn open_now(open: &Mutex<OpenRecognizer>) -> Opened {
    let opened = Instant::now();
    let result = (open.lock().unwrap_or_else(|e| e.into_inner()))();
    (result, opened.elapsed())
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

    fn now(&self) -> Instant {
        (self.clock)()
    }

    /// The models did work now.
    fn used(&mut self) {
        let now = self.now();
        self.keeper.used(now);
    }

    /// Starts a load of the speech model on its own thread, when it is not loaded and no load
    /// runs. The worker never waits for it: the result comes back as [`Job::Loaded`].
    fn load(&mut self, recognizer: &Loaded, why: Load) {
        if recognizer.is_some() || self.loading.is_some() {
            return;
        }
        self.loading = Some(why);
        let tx = self.me.clone();
        let open = self.open_recognizer.clone();
        let spawned = std::thread::Builder::new()
            .name("speech-in-load".into())
            .spawn(move || {
                let _ = tx.send(Job::Loaded(open_now(&open)));
            });
        if let Err(e) = spawned {
            let _ = self.me.send(Job::Loaded((
                Err(format!("The speech model load cannot start: {e}")),
                Duration::ZERO,
            )));
        }
    }

    /// A load ended. A recording that waits for it goes on, or fails with it. A preload that
    /// failed may have run before a download, so a recording that waits for it tries once more.
    fn loaded(&mut self, opened: Opened, recognizer: &mut Loaded) {
        let why = self.loading.take();
        let (result, took) = opened;
        match result {
            Ok(r) => {
                eprintln!("agentsi-pill: speech in ready in {took:?}");
                *recognizer = Some(r);
                self.used();
                self.load_detector();
            }
            Err(e) => {
                eprintln!("agentsi-pill: speech in cannot start: {e}");
                if why == Some(Load::Preload) && self.waiting.is_some() {
                    self.load(recognizer, Load::Press);
                    return;
                }
                if let Some(waiting) = self.waiting.take() {
                    self.abort();
                    self.ended(waiting.ticket, Err(e));
                }
            }
        }
        if std::mem::take(&mut self.shown) {
            self.send(Input::Loading(false));
        }
        let (Some(waiting), Some(r)) = (self.waiting.take(), recognizer.as_mut()) else {
            return;
        };
        let Waiting { ticket, kind, clip } = waiting;
        if let Err(e) = self.begin(ticket, kind) {
            eprintln!("agentsi-pill: cannot record: {e}");
            self.abort();
            self.ended(ticket, Err(e));
            return;
        }
        // Without a release yet, the recording goes on with the model.
        let Some(clip) = clip else {
            return;
        };
        let result = if kind == Kind::Hold {
            self.finish(r.as_mut(), Some(clip))
        } else {
            self.finish_toggle(r.as_mut(), Some(clip))
        };
        self.used();
        self.report(ticket, result);
    }

    fn report(&self, ticket: u64, result: Result<(), String>) {
        if let Err(e) = &result {
            eprintln!("agentsi-pill: dictation failed: {e}");
        }
        self.ended(ticket, result);
    }

    fn load_detector(&mut self) {
        match (self.open_detector)() {
            Ok(classifier) => self.cutter = Some(Cutter::new(classifier, PAUSE)),
            Err(e) => {
                eprintln!("agentsi-pill: voice detector cannot start: {e}");
                self.cutter = None;
            }
        }
    }

    /// Frees the models when the keep time passed since their last use. A recording keeps them.
    fn drop_if_due(&mut self, recognizer: &mut Loaded) {
        let now = self.now();
        if recognizer.is_none() || self.recording || !self.keeper.due(now) {
            return;
        }
        *recognizer = None;
        self.cutter = None;
        agentsi_voice::release_memory();
        self.keeper.dropped();
        eprintln!("{}", dropped_line("speech in", self.keeper.keep()));
    }

    /// The next job. A toggle dictation or a turn takes audio between the jobs, so it waits
    /// at most one poll: `Ok(None)` is a poll. An idle thread waits until the drop time of the
    /// models, or with no limit. `Err` when the dictator is gone.
    fn next_job(&mut self, recognizer: &mut Loaded) -> Result<Option<Job>, ()> {
        let limit = if self.live.is_some() {
            Some(self.poll)
        } else {
            self.drop_if_due(recognizer);
            if recognizer.is_some() && !self.recording {
                let now = self.now();
                self.keeper.wait(now)
            } else {
                None
            }
        };
        match limit {
            None => self.jobs.recv().map(Some).map_err(|_| ()),
            Some(limit) => match self.jobs.recv_timeout(limit) {
                Ok(job) => Ok(Some(job)),
                Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
                Err(mpsc::RecvTimeoutError::Disconnected) => Err(()),
            },
        }
    }

    fn run(mut self) {
        let mut recognizer: Loaded = None;
        if self.keeper.keep().loads_at_start() {
            self.load(&recognizer, Load::Preload);
        }
        loop {
            let Ok(job) = self.next_job(&mut recognizer) else {
                return;
            };
            let Some(job) = job else {
                if self.live.is_some()
                    && let Some(r) = recognizer.as_mut()
                {
                    self.tick(r.as_mut());
                    self.used();
                }
                continue;
            };
            match job {
                Job::Start { ticket, kind } => match self.start(ticket, kind, &recognizer) {
                    Ok(()) => self.used(),
                    Err(e) => {
                        eprintln!("agentsi-pill: cannot record: {e}");
                        self.ended(ticket, Err(e));
                    }
                },
                Job::Cancel => {
                    self.abort();
                    if recognizer.is_some() {
                        self.used();
                    }
                }
                Job::Keep(keep) => {
                    self.keeper.set(keep);
                    // A load that runs marks its use when it ends, so its drop time counts.
                    if keep.loads_at_start() {
                        self.load(&recognizer, Load::Preload);
                    }
                }
                Job::Loaded(opened) => self.loaded(opened, &mut recognizer),
                Job::Quit => return,
                Job::Finish(ticket) => {
                    if let Some(waiting) = self.waiting.as_mut() {
                        if waiting.ticket != ticket || waiting.clip.is_some() {
                            continue;
                        }
                        // The microphone closes at the release. The text waits for the model.
                        let clip = match self.mic.as_mut() {
                            Some(mic) => mic.finish(),
                            None => Err("The microphone is not open".into()),
                        };
                        self.recording = false;
                        match clip {
                            Ok(clip) => waiting.clip = Some(clip),
                            Err(e) => {
                                self.waiting = None;
                                self.report(ticket, Err(e));
                            }
                        }
                        continue;
                    }
                    // Without a model or a recording, the start failed already.
                    let (Some(r), true) = (recognizer.as_mut(), self.recording) else {
                        continue;
                    };
                    let running = self.live.as_ref().map(|l| l.ticket);
                    let result = match running {
                        Some(live) if live == ticket => self.finish_toggle(r.as_mut(), None),
                        // The stop of an old dictation: the one that runs goes on.
                        Some(_) => continue,
                        None => self.finish(r.as_mut(), None),
                    };
                    self.recording = false;
                    self.used();
                    self.report(ticket, result);
                }
            }
        }
    }

    /// Opens the microphone. Without the models, it starts their load and records while it
    /// runs.
    fn start(&mut self, ticket: u64, kind: Kind, recognizer: &Loaded) -> Result<(), String> {
        if self.mic.is_none() {
            self.mic = Some((self.open_mic)()?);
        }
        let mic = self.mic.as_mut().expect("set above");
        mic.start().inspect_err(|_| self.mic = None)?;
        self.recording = true;
        self.live = None;
        self.waiting = None;
        if recognizer.is_none() {
            self.waiting = Some(Waiting {
                ticket,
                kind,
                clip: None,
            });
            self.load(recognizer, Load::Press);
            if !self.shown {
                self.shown = true;
                self.send(Input::Loading(true));
            }
            return Ok(());
        }
        // A detector that was missing at an earlier load may be downloaded now.
        if kind != Kind::Hold && self.cutter.is_none() {
            self.load_detector();
        }
        self.begin(ticket, kind).inspect_err(|_| self.abort())
    }

    /// The models are ready: toggle dictation and a turn get the cutter.
    fn begin(&mut self, ticket: u64, kind: Kind) -> Result<(), String> {
        if kind == Kind::Hold {
            return Ok(());
        }
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
        let result = self.pump(recognizer, false, None);
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
        let pieces = match self.collect(false, None) {
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

    /// Ends the recording and drops what the microphone heard.
    fn abort(&mut self) {
        self.live = None;
        self.waiting = None;
        self.recording = false;
        if let Some(cutter) = self.cutter.as_mut() {
            cutter.reset();
        }
        if let Some(mic) = self.mic.as_mut() {
            mic.cancel();
        }
    }

    /// Takes the audio that came in since the last call, or with `last` the rest of it, and
    /// sends the pieces that it closes. `heard` is the rest when the microphone closed already.
    fn pump(
        &mut self,
        recognizer: &mut dyn Recognizer,
        last: bool,
        heard: Option<Clip>,
    ) -> Result<(), String> {
        for piece in self.collect(last, heard)? {
            self.send_piece(recognizer, &piece)?;
        }
        Ok(())
    }

    /// The pieces that the audio so far closes. With `last`, the rest of it closes the open one.
    fn collect(&mut self, last: bool, heard: Option<Clip>) -> Result<Vec<Vec<f32>>, String> {
        let clip = match heard {
            Some(clip) => clip,
            None => {
                let mic = self.mic.as_mut().ok_or("The microphone is not open")?;
                if last { mic.finish()? } else { mic.take()? }
            }
        };
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
    fn finish_toggle(
        &mut self,
        recognizer: &mut dyn Recognizer,
        heard: Option<Clip>,
    ) -> Result<(), String> {
        let result = self.pump(recognizer, true, heard);
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

    /// Capture is done: the clip goes through the model and the core. `heard` is the clip when
    /// the microphone closed already.
    fn finish(
        &mut self,
        recognizer: &mut dyn Recognizer,
        heard: Option<Clip>,
    ) -> Result<(), String> {
        let released = Instant::now();
        let clip = match heard {
            Some(clip) => clip,
            None => self
                .mic
                .as_mut()
                .ok_or("The microphone is not open")?
                .finish()?,
        };
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
            Policy::new(KeepTime::default()),
            false,
        );
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
                Ok(LinkEvent::Input(Input::Loading(_))) => {}
                Ok(LinkEvent::Input(input)) => return input,
                Ok(other) => panic!("unexpected {other:?}"),
                Err(_) => {
                    assert!(Instant::now() < deadline, "no event within 10 seconds");
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    /// No event came, but the news of a load.
    fn no_event(rx: &mut futures::channel::mpsc::UnboundedReceiver<LinkEvent>) -> bool {
        loop {
            match rx.try_recv() {
                Ok(LinkEvent::Input(Input::Loading(_))) => {}
                Ok(_) => return false,
                Err(_) => return true,
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
            Policy::new(KeepTime::default()),
            false,
        );
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

    /// A microphone whose audio the test gives by hand, as if the owner spoke.
    #[derive(Clone, Default)]
    struct LiveMic {
        heard: Arc<Mutex<Vec<f32>>>,
        recording: Arc<std::sync::atomic::AtomicBool>,
        cancels: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl LiveMic {
        fn open(&self) -> OpenMic {
            let mic = self.clone();
            Box::new(move || Ok(Box::new(mic.clone()) as Box<dyn Mic>))
        }

        fn speak(&self, samples: &[f32]) {
            assert!(self.is_recording(), "the microphone records");
            self.heard.lock().unwrap().extend_from_slice(samples);
        }

        fn is_recording(&self) -> bool {
            self.recording.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn clip(&self) -> Clip {
            Clip {
                samples: std::mem::take(&mut *self.heard.lock().unwrap()),
                rate: 16_000,
            }
        }
    }

    impl Mic for LiveMic {
        fn start(&mut self) -> Result<(), String> {
            self.heard.lock().unwrap().clear();
            self.recording
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        fn finish(&mut self) -> Result<Clip, String> {
            self.recording
                .store(false, std::sync::atomic::Ordering::SeqCst);
            Ok(self.clip())
        }
        fn take(&mut self) -> Result<Clip, String> {
            Ok(self.clip())
        }
        fn cancel(&mut self) {
            self.recording
                .store(false, std::sync::atomic::Ordering::SeqCst);
            self.cancels
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.heard.lock().unwrap().clear();
        }
    }

    /// The opens and drops of the fake models.
    #[derive(Clone, Default)]
    struct Models {
        opens: Arc<std::sync::atomic::AtomicUsize>,
        drops: Arc<std::sync::atomic::AtomicUsize>,
        detector_drops: Arc<std::sync::atomic::AtomicUsize>,
        lengths: Arc<Mutex<Vec<usize>>>,
    }

    impl Models {
        fn opens(&self) -> usize {
            self.opens.load(std::sync::atomic::Ordering::SeqCst)
        }
        fn drops(&self) -> usize {
            self.drops.load(std::sync::atomic::Ordering::SeqCst)
        }
        fn detector_drops(&self) -> usize {
            self.detector_drops
                .load(std::sync::atomic::Ordering::SeqCst)
        }

        /// A recognizer that waits for `gate` at each open, if one is given.
        fn recognizer(&self, gate: Option<mpsc::Receiver<()>>) -> OpenRecognizer {
            let me = self.clone();
            let gate = gate.map(Mutex::new);
            Box::new(move || {
                me.opens.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if let Some(gate) = &gate {
                    gate.lock().unwrap().recv().unwrap();
                }
                Ok(Box::new(Counted(me.clone())) as Box<dyn Recognizer>)
            })
        }

        /// A recognizer whose open takes `load`.
        fn slow(&self, load: Duration) -> OpenRecognizer {
            let me = self.clone();
            Box::new(move || {
                me.opens.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                std::thread::sleep(load);
                Ok(Box::new(Counted(me.clone())) as Box<dyn Recognizer>)
            })
        }

        fn detector(&self) -> OpenDetector {
            let me = self.clone();
            Box::new(move || Ok(Box::new(CountedLoud(me.clone())) as Box<dyn Classifier + Send>))
        }
    }

    /// Names each clip "p1", "p2" and keeps its length.
    struct Counted(Models);

    impl Recognizer for Counted {
        fn transcribe(&mut self, samples: &[f32]) -> Result<String, String> {
            let mut lengths = self.0.lengths.lock().unwrap();
            lengths.push(samples.len());
            Ok(format!("p{}", lengths.len()))
        }
    }

    impl Drop for Counted {
        fn drop(&mut self) {
            self.0
                .drops
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    struct CountedLoud(Models);

    impl Classifier for CountedLoud {
        fn speech(&mut self, frame: &[f32]) -> Result<f32, vad::Error> {
            Loud.speech(frame)
        }
        fn reset(&mut self) {}
    }

    impl Drop for CountedLoud {
        fn drop(&mut self) {
            self.0
                .detector_drops
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    struct Pill {
        dictator: Dictator,
        rx: futures::channel::mpsc::UnboundedReceiver<LinkEvent>,
        mic: LiveMic,
        models: Models,
        log: Arc<Mutex<Log>>,
    }

    fn pill(policy: Policy, gate: Option<mpsc::Receiver<()>>) -> Pill {
        let (tx, rx) = unbounded();
        let mic = LiveMic::default();
        let models = Models::default();
        let log = Arc::new(Mutex::new(Log::default()));
        let dictator = Dictator::start_polling(
            Duration::from_millis(1),
            mic.open(),
            models.recognizer(gate),
            models.detector(),
            Box::new(Sink {
                log: log.clone(),
                terminal: Some("t1".into()),
            }),
            tx,
            policy,
            false,
        );
        Pill {
            dictator,
            rx,
            mic,
            models,
            log,
        }
    }

    fn hold(ticket: u64) -> Effect {
        Effect::StartRecording {
            ticket,
            toggle: false,
        }
    }

    #[test]
    fn a_model_that_cannot_load_fails_at_the_key_press_and_closes_the_microphone() {
        let (tx, mut rx) = unbounded();
        let mic = LiveMic::default();
        let dictator = Dictator::start(
            mic.open(),
            Box::new(|| Err(NOT_DOWNLOADED.into())),
            no_detector(),
            Box::new(Sink {
                log: Arc::default(),
                terminal: None,
            }),
            tx,
            Policy::new(KeepTime::default()),
            false,
        );
        dictator.effect(&hold(1));
        assert_eq!(next(&mut rx), failed(1, NOT_DOWNLOADED));
        assert!(!mic.is_recording());
        assert_eq!(mic.cancels.load(std::sync::atomic::Ordering::SeqCst), 1);
        // The release that follows reports nothing more.
        dictator.effect(&Effect::FinishRecording { ticket: 1 });
        std::thread::sleep(Duration::from_millis(50));
        assert!(no_event(&mut rx));
    }

    #[test]
    fn a_new_pill_holds_no_model_and_the_first_dictation_loses_no_audio_while_it_loads() {
        let (release, gate) = mpsc::channel::<()>();
        let mut p = pill(Policy::new(KeepTime::FiveMinutes), Some(gate));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(p.models.opens(), 0, "no model at the start");
        p.dictator.effect(&hold(1));
        // The model loads, and the microphone records already.
        wait_for("the load", || p.models.opens() == 1);
        wait_for("the microphone", || p.mic.is_recording());
        p.mic.speak(&speech(1.5));
        p.dictator.effect(&Effect::FinishRecording { ticket: 1 });
        std::thread::sleep(Duration::from_millis(50));
        assert!(no_event(&mut p.rx), "the text waits for the model");
        release.send(()).unwrap();
        assert_eq!(next(&mut p.rx), done(1));
        assert_eq!(
            *p.models.lengths.lock().unwrap(),
            [24_000],
            "the whole clip"
        );
        assert_eq!(p.log.lock().unwrap().delivered, ["p1"]);
    }

    #[test]
    fn toggle_dictation_and_a_turn_lose_no_audio_while_the_model_loads() {
        // Toggle dictation: the speech before the stop is one piece.
        let (release, gate) = mpsc::channel::<()>();
        let mut p = pill(Policy::new(KeepTime::FiveMinutes), Some(gate));
        p.dictator.effect(&start_toggle(1));
        wait_for("the microphone", || p.mic.is_recording());
        p.mic.speak(&speech(2.0));
        p.dictator.effect(&Effect::FinishRecording { ticket: 1 });
        release.send(()).unwrap();
        assert_eq!(next(&mut p.rx), done(1));
        assert_eq!(p.log.lock().unwrap().delivered, ["p1"]);
        // The cutter works in whole frames, so only the last part frame may be missing.
        let piece = p.models.lengths.lock().unwrap()[0];
        assert!(piece > 32_000 - vad::FRAME, "{piece}");
        // A turn of conversation mode: the speech and the pause that ends it.
        let (release, gate) = mpsc::channel::<()>();
        let mut p = pill(Policy::new(KeepTime::FiveMinutes), Some(gate));
        p.dictator.effect(&Effect::Listen { ticket: 2 });
        wait_for("the microphone", || p.mic.is_recording());
        p.mic.speak(&[speech(1.0), quiet(2.0)].concat());
        release.send(()).unwrap();
        assert_eq!(next(&mut p.rx), heard(2));
        assert_eq!(next(&mut p.rx), outcome(2, Turn::Sent));
        let turn = p.models.lengths.lock().unwrap()[0];
        assert!(turn >= 16_000, "{turn}");
    }

    #[test]
    fn with_a_keep_time_of_1_minute_unused_models_drop_and_a_recording_keeps_them() {
        let clock = crate::keep::FakeClock::new();
        let mut p = pill(clock.policy(KeepTime::OneMinute), None);
        p.dictator.effect(&hold(1));
        wait_for("the microphone", || p.mic.is_recording());
        p.mic.speak(&speech(1.0));
        p.dictator.effect(&Effect::FinishRecording { ticket: 1 });
        assert_eq!(next(&mut p.rx), done(1));
        // A recording that lasts past the keep time keeps the models.
        p.dictator.effect(&hold(2));
        wait_for("the microphone", || p.mic.is_recording());
        clock.advance(Duration::from_secs(120));
        p.dictator.set_keep(KeepTime::OneMinute);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(p.models.drops(), 0, "a model in use stays");
        p.mic.speak(&speech(1.0));
        p.dictator.effect(&Effect::FinishRecording { ticket: 2 });
        assert_eq!(next(&mut p.rx), done(2));
        // 59 s after the last use the models stay. 61 s after it they drop together.
        clock.advance(Duration::from_secs(59));
        p.dictator.set_keep(KeepTime::OneMinute);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(p.models.drops(), 0);
        clock.advance(Duration::from_secs(2));
        p.dictator.set_keep(KeepTime::OneMinute);
        wait_for("the drop", || p.models.drops() == 1);
        assert_eq!(p.models.detector_drops(), 1);
        // The next dictation loads them again.
        p.dictator.effect(&hold(3));
        wait_for("the microphone", || p.mic.is_recording());
        p.mic.speak(&speech(1.0));
        p.dictator.effect(&Effect::FinishRecording { ticket: 3 });
        assert_eq!(next(&mut p.rx), done(3));
        assert_eq!(p.models.opens(), 2);
    }

    #[test]
    fn an_idle_dictator_wakes_only_at_the_drop_time() {
        let clock = crate::keep::FakeClock::new();
        // With no model loaded, the thread waits with no limit.
        let mut p = pill(clock.policy(KeepTime::OneMinute), None);
        std::thread::sleep(Duration::from_millis(200));
        let reads = clock.reads();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(clock.reads(), reads);
        // With the models loaded, it waits for their drop time and reads no clock before.
        p.dictator.effect(&hold(1));
        wait_for("the microphone", || p.mic.is_recording());
        p.mic.speak(&speech(1.0));
        p.dictator.effect(&Effect::FinishRecording { ticket: 1 });
        assert_eq!(next(&mut p.rx), done(1));
        std::thread::sleep(Duration::from_millis(200));
        let reads = clock.reads();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(clock.reads(), reads, "no wake before the drop time");
    }

    #[test]
    fn always_keeps_the_models_and_load_at_start_loads_them_at_once() {
        let clock = crate::keep::FakeClock::new();
        let mut p = pill(clock.policy(KeepTime::Always), None);
        assert_eq!(p.models.opens(), 0);
        p.dictator.effect(&hold(1));
        wait_for("the microphone", || p.mic.is_recording());
        p.mic.speak(&speech(1.0));
        p.dictator.effect(&Effect::FinishRecording { ticket: 1 });
        assert_eq!(next(&mut p.rx), done(1));
        clock.advance(Duration::from_secs(24 * 3600));
        p.dictator.set_keep(KeepTime::Always);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(p.models.drops(), 0);
        let at_start = pill(clock.policy(KeepTime::Start), None);
        wait_for("the load at start", || at_start.models.opens() == 1);
        clock.advance(Duration::from_secs(24 * 3600));
        at_start.dictator.set_keep(KeepTime::Start);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(at_start.models.drops(), 0);
        assert_eq!(at_start.models.opens(), 1);
    }

    #[test]
    fn a_new_keep_time_applies_with_no_restart() {
        let clock = crate::keep::FakeClock::new();
        let mut p = pill(clock.policy(KeepTime::FiveMinutes), None);
        // "Load at start" chosen while the pill runs loads the models now.
        p.dictator.set_keep(KeepTime::Start);
        wait_for("the load", || p.models.opens() == 1);
        p.dictator.effect(&hold(1));
        wait_for("the microphone", || p.mic.is_recording());
        p.mic.speak(&speech(1.0));
        p.dictator.effect(&Effect::FinishRecording { ticket: 1 });
        assert_eq!(next(&mut p.rx), done(1));
        assert_eq!(p.models.opens(), 1, "the dictation used the loaded model");
        // A keep time of 1 minute, 2 minutes after the last use, drops them at once.
        clock.advance(Duration::from_secs(120));
        p.dictator.set_keep(KeepTime::OneMinute);
        wait_for("the drop", || p.models.drops() == 1);
        assert_eq!(p.models.detector_drops(), 1);
    }

    #[test]
    fn a_press_while_the_models_load_at_start_loses_no_audio() {
        let (release, gate) = mpsc::channel::<()>();
        let mut p = pill(Policy::new(KeepTime::Start), Some(gate));
        wait_for("the load at start", || p.models.opens() == 1);
        p.dictator.effect(&hold(1));
        // The load still waits at the gate, and the microphone records already.
        wait_for("the microphone", || p.mic.is_recording());
        p.mic.speak(&speech(1.5));
        p.dictator.effect(&Effect::FinishRecording { ticket: 1 });
        release.send(()).unwrap();
        assert_eq!(next(&mut p.rx), done(1));
        assert_eq!(*p.models.lengths.lock().unwrap(), [24_000]);
        assert_eq!(p.models.opens(), 1);
    }

    fn slow_pill(load: Duration) -> Pill {
        let (tx, rx) = unbounded();
        let mic = LiveMic::default();
        let models = Models::default();
        let log = Arc::new(Mutex::new(Log::default()));
        let dictator = Dictator::start(
            mic.open(),
            models.slow(load),
            models.detector(),
            Box::new(Sink {
                log: log.clone(),
                terminal: Some("t1".into()),
            }),
            tx,
            Policy::new(KeepTime::FiveMinutes),
            false,
        );
        Pill {
            dictator,
            rx,
            mic,
            models,
            log,
        }
    }

    #[test]
    fn a_release_during_a_slow_load_closes_the_microphone_and_keeps_only_the_audio_before_it() {
        let mut p = slow_pill(Duration::from_secs(3));
        p.dictator.effect(&hold(1));
        wait_for("the microphone", || p.mic.is_recording());
        p.mic.speak(&speech(0.5));
        let released = Instant::now();
        p.dictator.effect(&Effect::FinishRecording { ticket: 1 });
        wait_for("the release", || !p.mic.is_recording());
        assert!(
            released.elapsed() < Duration::from_secs(1),
            "closed at once"
        );
        // Sound after the release must not reach the text.
        p.mic.heard.lock().unwrap().extend(speech(1.0));
        assert_eq!(next(&mut p.rx), done(1));
        assert_eq!(*p.models.lengths.lock().unwrap(), [8_000]);
        assert_eq!(p.log.lock().unwrap().delivered, ["p1"]);
    }

    #[test]
    fn a_cancel_during_a_slow_load_closes_the_microphone_at_once_and_reports_nothing() {
        let mut p = slow_pill(Duration::from_secs(3));
        p.dictator.effect(&hold(1));
        wait_for("the microphone", || p.mic.is_recording());
        p.mic.speak(&speech(0.5));
        let cancelled = Instant::now();
        p.dictator.effect(&Effect::CancelRecording);
        wait_for("the cancel", || !p.mic.is_recording());
        assert!(
            cancelled.elapsed() < Duration::from_secs(1),
            "closed at once"
        );
        assert_eq!(p.mic.cancels.load(std::sync::atomic::Ordering::SeqCst), 1);
        // The load ends with no text: the next press uses the loaded model.
        wait_for("the load", || p.models.opens() == 1);
        std::thread::sleep(Duration::from_millis(3_200));
        assert!(no_event(&mut p.rx));
        assert!(p.models.lengths.lock().unwrap().is_empty());
        p.dictator.effect(&hold(2));
        wait_for("the microphone", || p.mic.is_recording());
        p.mic.speak(&speech(0.5));
        p.dictator.effect(&Effect::FinishRecording { ticket: 2 });
        assert_eq!(next(&mut p.rx), done(2));
        assert_eq!(p.models.opens(), 1);
    }

    #[test]
    fn a_timed_keep_during_a_load_at_start_does_not_hold_back_a_press() {
        let (release, gate) = mpsc::channel::<()>();
        let mut p = pill(Policy::new(KeepTime::Start), Some(gate));
        wait_for("the load at start", || p.models.opens() == 1);
        p.dictator.set_keep(KeepTime::OneMinute);
        p.dictator.effect(&hold(1));
        // The load still waits at the gate, and the microphone records already.
        wait_for("the microphone", || p.mic.is_recording());
        p.mic.speak(&speech(1.0));
        p.dictator.effect(&Effect::FinishRecording { ticket: 1 });
        wait_for("the release", || !p.mic.is_recording());
        release.send(()).unwrap();
        assert_eq!(next(&mut p.rx), done(1));
        assert_eq!(*p.models.lengths.lock().unwrap(), [16_000]);
        assert_eq!(p.models.opens(), 1);
    }

    #[test]
    fn a_load_at_start_that_failed_before_a_download_does_not_fail_the_next_press() {
        // The first open fails, as with no model on disk. A second open finds it.
        let fails = |gate: Option<mpsc::Receiver<()>>| -> (OpenRecognizer, Arc<Mutex<u32>>) {
            let opens = Arc::new(Mutex::new(0));
            let n = opens.clone();
            let gate = gate.map(Mutex::new);
            let open: OpenRecognizer = Box::new(move || {
                let first = {
                    let mut n = n.lock().unwrap();
                    *n += 1;
                    *n == 1
                };
                if first {
                    if let Some(gate) = &gate {
                        gate.lock().unwrap().recv().unwrap();
                    }
                    return Err(NOT_DOWNLOADED.into());
                }
                Ok(Box::new(Words("Hello.")) as Box<dyn Recognizer>)
            });
            (open, opens)
        };
        let start = |open: OpenRecognizer| {
            let (tx, rx) = unbounded();
            let mic = LiveMic::default();
            let dictator = Dictator::start(
                mic.open(),
                open,
                no_detector(),
                Box::new(Sink {
                    log: Arc::default(),
                    terminal: Some("t1".into()),
                }),
                tx,
                Policy::new(KeepTime::Start),
                false,
            );
            (dictator, rx, mic)
        };
        // The failed load ended before the press.
        let (open, opens) = fails(None);
        let (dictator, mut rx, mic) = start(open);
        wait_for("the load at start", || *opens.lock().unwrap() == 1);
        std::thread::sleep(Duration::from_millis(50));
        dictator.effect(&hold(1));
        wait_for("the microphone", || mic.is_recording());
        mic.speak(&speech(0.5));
        dictator.effect(&Effect::FinishRecording { ticket: 1 });
        assert_eq!(next(&mut rx), done(1));
        assert_eq!(*opens.lock().unwrap(), 2);
        // The failed load ends while the press waits for it: it opens once more.
        let (release, gate) = mpsc::channel::<()>();
        let (open, opens) = fails(Some(gate));
        let (dictator, mut rx, mic) = start(open);
        wait_for("the load at start", || *opens.lock().unwrap() == 1);
        dictator.effect(&hold(1));
        wait_for("the microphone", || mic.is_recording());
        mic.speak(&speech(0.5));
        dictator.effect(&Effect::FinishRecording { ticket: 1 });
        release.send(()).unwrap();
        assert_eq!(next(&mut rx), done(1));
        assert_eq!(*opens.lock().unwrap(), 2);
    }

    #[test]
    fn a_press_that_waits_for_the_model_tells_the_pill_of_the_load() {
        let (release, gate) = mpsc::channel::<()>();
        let mut p = pill(Policy::new(KeepTime::FiveMinutes), Some(gate));
        p.dictator.effect(&hold(1));
        wait_for("the microphone", || p.mic.is_recording());
        let event = |rx: &mut futures::channel::mpsc::UnboundedReceiver<LinkEvent>| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Ok(LinkEvent::Input(input)) = rx.try_recv() {
                    return input;
                }
                assert!(Instant::now() < deadline, "no event");
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        assert_eq!(event(&mut p.rx), Input::Loading(true));
        p.mic.speak(&speech(0.5));
        p.dictator.effect(&Effect::FinishRecording { ticket: 1 });
        release.send(()).unwrap();
        assert_eq!(event(&mut p.rx), Input::Loading(false));
        assert_eq!(event(&mut p.rx), done(1));
        // With the model loaded, a press says nothing of a load.
        p.dictator.effect(&hold(2));
        wait_for("the microphone", || p.mic.is_recording());
        p.mic.speak(&speech(0.5));
        p.dictator.effect(&Effect::FinishRecording { ticket: 2 });
        assert_eq!(event(&mut p.rx), done(2));
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
            Policy::new(KeepTime::default()),
            false,
        );
        dictator.effect(&Effect::StartRecording {
            ticket: 1,
            toggle: false,
        });
        assert_eq!(next(&mut rx), failed(1, NOT_DOWNLOADED));
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
            Policy::new(KeepTime::default()),
            false,
        );
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
            Policy::new(KeepTime::default()),
            false,
        );
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
        assert!(no_event(&mut rx));
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
        assert!(no_event(&mut rx));
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
        assert!(no_event(&mut rx));
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
            Policy::new(KeepTime::default()),
            false,
        );
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
                Policy::new(KeepTime::default()),
                false,
            );
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
        assert!(no_event(&mut rx));
    }
}
