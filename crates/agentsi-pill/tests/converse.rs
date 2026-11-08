//! Conversation mode, from the key to the pane and back, on a real core.
//!
//! The path is the real one after the microphone: `agentsi-core converse` makes the core send
//! `converse.state`, the pill link turns it into inputs, the state model gives effects, and the
//! dictator cuts each turn at a pause of 1.2 s, transcribes it and calls `voice.transcript`.
//! The core submits the turn to the target pane. The pane is fake: it runs `cat` on a raw
//! terminal and writes each byte it gets to a file, so the test counts the Returns. The test
//! plays the hooks of Claude Code (`hook.event`), as the bridge of the pane would, and a fake
//! speaker that plays each reply for a moment.
//!
//! One test only: it sets environment variables that the whole process shares. The microphone,
//! the speech model and the voice detector are fakes, so the test always runs.

use agentsi_core::server::{bind, euid};
use agentsi_core::state::Core;
use agentsi_pill::dictate::{CoreDelivery, Dictator, Mic, OpenMic, Recognizer};
use agentsi_pill::link::{self, LinkEvent, LinkSender};
use agentsi_pill::state::{Effect, Input, Model, Phase, SpeechEvent};
use agentsi_proto::client::Client;
use agentsi_proto::socket::SocketPath;
use agentsi_proto::terminal_api::{Create, CreateParams, Kill, TerminalId};
use agentsi_proto::voice_api::{
    FocusReport, FocusReportParams, HookEvent, HookEventParams, HookKind, Mode, ModeParams,
};
use agentsi_voice::stt::Clip;
use agentsi_voice::vad::{self, Classifier};
use futures::channel::mpsc::unbounded;
use serde_json::json;
use std::collections::VecDeque;
use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn wait_for<T>(what: &str, mut ok: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(v) = ok() {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn start_core(dir: &Path) -> SocketPath {
    let socket = SocketPath {
        path: dir.join("run").join("core.sock"),
        default: false,
    };
    let bound = bind(&socket).unwrap();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let _ = bound
                .serve(Core::new(), euid(), std::future::pending::<()>())
                .await;
        });
    });
    wait_for("the core listens", || {
        std::os::unix::net::UnixStream::connect(&socket.path).ok()
    });
    socket
}

/// A fake pane that writes the exact bytes it gets to `out`.
fn start_pane(client: &mut Client, dir: &Path, out: &Path, focus: bool) -> String {
    let id = client
        .call::<Create>(&CreateParams {
            cwd: dir.display().to_string(),
            argv: vec![
                "sh".into(),
                "-c".into(),
                format!("stty raw -echo; exec cat > '{}'", out.display()),
            ],
            env: Default::default(),
            product: None,
            cols: None,
            rows: None,
        })
        .unwrap()
        .id;
    if focus {
        client
            .call::<FocusReport>(&FocusReportParams {
                terminal: Some(id.clone()),
            })
            .unwrap();
    }
    id
}

fn voice_on(client: &mut Client, id: &str) -> bool {
    client
        .call::<Mode>(&ModeParams {
            terminal: id.into(),
            on: None,
        })
        .unwrap()
        .on
}

/// The hooks of one turn of Claude Code: the prompt, then the reply.
fn reply(client: &mut Client, id: &str, prompt: &str, message: &str) {
    for (event, payload) in [
        (HookKind::UserPromptSubmit, json!({ "prompt": prompt })),
        (HookKind::Stop, json!({ "last_assistant_message": message })),
    ] {
        client
            .call::<HookEvent>(&HookEventParams {
                terminal: id.into(),
                session: "fake".into(),
                event,
                payload,
            })
            .unwrap();
    }
}

fn speech(seconds: f32) -> Vec<f32> {
    (0..(seconds * 16_000.0) as usize)
        .map(|i| if i % 2 == 0 { 0.5 } else { -0.5 })
        .collect()
}

/// One turn of audio: a second of speech, then a pause that ends the turn.
fn turn_audio() -> Vec<f32> {
    [speech(1.0), vec![0.0; 16_000 * 2]].concat()
}

/// A microphone that plays the next turn of the script at each start. Audio that comes while
/// it is closed is lost, as with a real one.
struct TurnMic {
    turns: Arc<Mutex<VecDeque<Vec<f32>>>>,
    playing: Vec<f32>,
    at: usize,
    open: Arc<AtomicBool>,
    opens: Arc<AtomicUsize>,
}

impl Mic for TurnMic {
    fn start(&mut self) -> Result<(), String> {
        self.playing = self.turns.lock().unwrap().pop_front().unwrap_or_default();
        self.at = 0;
        self.open.store(true, Ordering::SeqCst);
        self.opens.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn take(&mut self) -> Result<Clip, String> {
        let end = (self.at + 8_000).min(self.playing.len());
        let samples = self.playing[self.at..end].to_vec();
        self.at = end;
        Ok(Clip {
            samples,
            rate: 16_000,
        })
    }
    fn finish(&mut self) -> Result<Clip, String> {
        self.open.store(false, Ordering::SeqCst);
        Ok(Clip {
            samples: Vec::new(),
            rate: 16_000,
        })
    }
    fn cancel(&mut self) {
        self.open.store(false, Ordering::SeqCst);
    }
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

/// Gives the next text of the script for each turn.
struct Script(Arc<Mutex<VecDeque<&'static str>>>);

impl Recognizer for Script {
    fn transcribe(&mut self, _: &[f32]) -> Result<String, String> {
        Ok(self.0.lock().unwrap().pop_front().unwrap_or("").to_owned())
    }
}

/// The pill without its window: the link, the state model, the dictator, and a speaker that
/// plays each reply for `SPEECH` and says so.
struct Pill {
    phases: Arc<Mutex<Vec<Phase>>>,
    tx: LinkSender,
    /// The microphone is open now.
    mic_open: Arc<AtomicBool>,
    /// How many times the microphone opened.
    mic_opens: Arc<AtomicUsize>,
    /// The microphone was open while a reply played.
    heard_the_voice: Arc<AtomicBool>,
    /// The replies that played to the end.
    spoken: Arc<AtomicUsize>,
}

const SPEECH: Duration = Duration::from_millis(500);

fn start_pill(
    turns: Arc<Mutex<VecDeque<Vec<f32>>>>,
    texts: Arc<Mutex<VecDeque<&'static str>>>,
) -> Pill {
    let (tx, mut rx) = unbounded::<LinkEvent>();
    link::spawn(tx.clone());
    let mic_open = Arc::new(AtomicBool::new(false));
    let mic_opens = Arc::new(AtomicUsize::new(0));
    let (open, opens) = (mic_open.clone(), mic_opens.clone());
    let open_mic: OpenMic = Box::new(move || {
        Ok(Box::new(TurnMic {
            turns: turns.clone(),
            playing: Vec::new(),
            at: 0,
            open: open.clone(),
            opens: opens.clone(),
        }) as Box<dyn Mic>)
    });
    let dictator = Arc::new(Dictator::start_polling(
        Duration::from_millis(1),
        open_mic,
        Box::new(move || Ok(Box::new(Script(texts.clone())) as Box<dyn Recognizer>)),
        Box::new(|| Ok(Box::new(Loud) as Box<dyn Classifier + Send>)),
        Box::new(CoreDelivery),
        tx.clone(),
        false,
    ));
    // A press while the model loads is refused, so the test waits for the model.
    assert!(dictator.wait_until_loaded(Duration::from_secs(30)));
    let phases = Arc::new(Mutex::new(vec![Phase::Hidden]));
    let heard_the_voice = Arc::new(AtomicBool::new(false));
    let spoken = Arc::new(AtomicUsize::new(0));
    let (seen, link_tx, open, heard, done) = (
        phases.clone(),
        tx.clone(),
        mic_open.clone(),
        heard_the_voice.clone(),
        spoken.clone(),
    );
    std::thread::spawn(move || {
        let mut model = Model::default();
        while let Some(event) = futures::executor::block_on(futures::StreamExt::next(&mut rx)) {
            let LinkEvent::Input(input) = event else {
                continue;
            };
            for effect in model.apply(input) {
                match &effect {
                    Effect::StopRecording => link::stop_recording(link_tx.clone()),
                    Effect::EndConversation => link::stop_conversation(link_tx.clone()),
                    Effect::StartSpeech { ticket, .. } => {
                        let (tx, open, heard, done, ticket) = (
                            link_tx.clone(),
                            open.clone(),
                            heard.clone(),
                            done.clone(),
                            *ticket,
                        );
                        std::thread::spawn(move || {
                            let say = |event| {
                                let _ = tx.unbounded_send(LinkEvent::Input(Input::Speech {
                                    ticket,
                                    event,
                                }));
                            };
                            say(SpeechEvent::Playing);
                            let end = Instant::now() + SPEECH;
                            while Instant::now() < end {
                                if open.load(Ordering::SeqCst) {
                                    heard.store(true, Ordering::SeqCst);
                                }
                                std::thread::sleep(Duration::from_millis(5));
                            }
                            done.fetch_add(1, Ordering::SeqCst);
                            say(SpeechEvent::Done);
                        });
                    }
                    _ => {}
                }
                dictator.effect(&effect);
            }
            let mut seen = seen.lock().unwrap();
            if seen.last() != Some(&model.phase()) {
                seen.push(model.phase());
            }
        }
    });
    Pill {
        phases,
        tx,
        mic_open,
        mic_opens,
        heard_the_voice,
        spoken,
    }
}

impl Pill {
    fn phase(&self) -> Phase {
        *self.phases.lock().unwrap().last().unwrap()
    }

    fn wait_phase(&self, phase: Phase) {
        wait_for(&format!("the pill shows {phase:?}"), || {
            (self.phase() == phase).then_some(())
        });
    }
}

fn converse(action: &str) -> ExitCode {
    agentsi_core::converse::run(&[action])
}

fn returns(out: &Path) -> usize {
    std::fs::read(out)
        .unwrap_or_default()
        .iter()
        .filter(|b| **b == b'\r')
        .count()
}

#[test]
fn three_turns_reach_the_pane_the_voice_is_not_heard_and_stop_ends_the_mode() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // SAFETY: this is the only test of this process, and no other thread runs yet.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", root.join("config"));
        std::env::set_var("AGENTSI_HOME", root.join("home"));
        std::env::set_var("HOME", root.join("home"));
    }
    std::fs::create_dir_all(root.join("home")).unwrap();
    let socket = start_core(root);
    // SAFETY: as above. The core runs in this process but reads no environment at this point.
    unsafe { std::env::set_var("AGENTSI_CORE_SOCKET", &socket.path) };

    let mut client = Client::connect(&socket.path).unwrap();
    let out = root.join("pane.bytes");
    let id = start_pane(&mut client, root, &out, true);
    assert!(!voice_on(&mut client, &id));

    // The script: turn 1, turn 2 with no text, turn 3, a turn heard while the first reply plays
    // (it must be lost), turn 4, the word "stop". Each start of the microphone plays one turn.
    let turns: Arc<Mutex<VecDeque<Vec<f32>>>> =
        Arc::new(Mutex::new((0..6).map(|_| turn_audio()).collect()));
    let texts = Arc::new(Mutex::new(VecDeque::from([
        "open the second story",
        "",
        "and approve it",
        "what is left",
        "Stop.",
    ])));
    let pill = start_pill(turns.clone(), texts.clone());

    // The first press can come before this pill subscribes, and the core then refuses it. A
    // second start for the same pane changes nothing, so the test starts until the pill listens.
    wait_for("the pill listens", || {
        let _ = converse("start");
        std::thread::sleep(Duration::from_millis(100));
        (pill.phase() != Phase::Hidden).then_some(())
    });
    // Voice mode of the pane is on for the mode. The first turn follows with no key.
    assert!(voice_on(&mut client, &id));

    // Turn 1 reaches the pane with one Return, and the pill waits. The next turn of the script
    // is lost: it plays only when the microphone opens again.
    wait_for("turn 1 is in the pane", || {
        (std::fs::read(&out).ok()? == b"open the second story\r").then_some(())
    });
    pill.wait_phase(Phase::Waiting);
    let opens = pill.mic_opens.load(Ordering::SeqCst);
    assert_eq!(opens, 1);
    assert!(
        !pill.mic_open.load(Ordering::SeqCst),
        "the pane works: no audio"
    );
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(pill.mic_opens.load(Ordering::SeqCst), opens);
    assert_eq!(
        turns.lock().unwrap().len(),
        5,
        "no audio taken while it waits"
    );

    // The reply is spoken. The microphone stays closed until its last sentence ends.
    reply(
        &mut client,
        &id,
        "open the second story",
        "Opened. It has two tasks.",
    );
    wait_for("the voice speaks", || {
        pill.phases
            .lock()
            .unwrap()
            .contains(&Phase::Speaking)
            .then_some(())
    });
    assert!(
        !pill.mic_open.load(Ordering::SeqCst),
        "the voice speaks: no audio"
    );
    wait_for("the reply played to the end", || {
        (pill.spoken.load(Ordering::SeqCst) == 1).then_some(())
    });
    assert!(!pill.heard_the_voice.load(Ordering::SeqCst));

    // Turn 2 has no text: nothing is sent, and the pill listens again. Turn 3 is sent.
    wait_for("turn 3 is in the pane", || {
        (std::fs::read(&out).ok()? == b"open the second story\rand approve it\r").then_some(())
    });
    assert_eq!(returns(&out), 2);
    pill.wait_phase(Phase::Waiting);

    // A reply with nothing to speak: the pill listens again with no speech.
    reply(&mut client, &id, "and approve it", "   ");
    wait_for("turn 4 is in the pane", || {
        (returns(&out) == 3).then_some(())
    });
    assert_eq!(
        std::fs::read(&out).unwrap(),
        b"open the second story\rand approve it\rwhat is left\r"
    );
    assert_eq!(pill.spoken.load(Ordering::SeqCst), 1, "nothing was spoken");
    pill.wait_phase(Phase::Waiting);

    // The word "stop" alone ends the mode. It is not sent, and voice mode is back off.
    reply(&mut client, &id, "what is left", "Nothing.");
    wait_for("the mode ends", || {
        (pill.phase() == Phase::Hidden && pill.spoken.load(Ordering::SeqCst) == 2).then_some(())
    });
    assert!(!voice_on(&mut client, &id), "voice mode is back as it was");
    assert_eq!(returns(&out), 3, "the word stop is not sent");
    assert!(!pill.mic_open.load(Ordering::SeqCst));
    assert!(!pill.heard_the_voice.load(Ordering::SeqCst));
    let seen = pill.phases.lock().unwrap().clone();
    assert!(
        seen.windows(4).any(|w| w
            == [
                Phase::Listening,
                Phase::Transcribing,
                Phase::Waiting,
                Phase::Preparing
            ]),
        "{seen:?}"
    );

    // A second press of the key (toggle) starts again, and a click on the pill ends it.
    texts.lock().unwrap().push_back("one more");
    turns.lock().unwrap().push_back(turn_audio());
    wait_for("the pill listens", || {
        let _ = converse("start");
        std::thread::sleep(Duration::from_millis(100));
        (pill.phase() != Phase::Hidden).then_some(())
    });
    wait_for("turn 5 is in the pane", || {
        (returns(&out) == 4).then_some(())
    });
    pill.wait_phase(Phase::Waiting);
    assert!(voice_on(&mut client, &id));
    pill.tx
        .unbounded_send(LinkEvent::Input(Input::Click))
        .unwrap();
    pill.wait_phase(Phase::Hidden);
    assert!(!voice_on(&mut client, &id));

    // The key ends the mode too, and a pane that closes ends it for the pill.
    wait_for("the pill listens", || {
        let _ = converse("start");
        std::thread::sleep(Duration::from_millis(100));
        (pill.phase() != Phase::Hidden).then_some(())
    });
    assert_eq!(converse("toggle"), ExitCode::SUCCESS);
    pill.wait_phase(Phase::Hidden);
    assert!(!voice_on(&mut client, &id));
    wait_for("the pill listens", || {
        let _ = converse("start");
        std::thread::sleep(Duration::from_millis(100));
        (pill.phase() != Phase::Hidden).then_some(())
    });
    client.call::<Kill>(&TerminalId { id }).unwrap();
    pill.wait_phase(Phase::Hidden);
    assert!(!pill.mic_open.load(Ordering::SeqCst));
}
