//! A WAV file goes through dictation into a real pane of a throwaway core.
//!
//! The path is the real one after the capture step: the key (`agentsi-core dictate start`
//! and `stop`) makes the core send `dictation.*` events, the pill link turns them into
//! inputs, the state model gives effects, and the dictator resamples, transcribes and
//! calls `voice.transcript`. The core applies the vocabulary and types into the pane.
//! The pane runs `cat` on a raw terminal and writes each byte it gets to a file, so the
//! test sees the exact bytes: the text, and no Return.
//!
//! One test only: it sets environment variables that the whole process shares.
//! The first stage uses a fake model and always runs. The second stage uses Parakeet and
//! runs when the model files are there (`AGENTSI_STT_DIR`, else the standard folder).
//! The third stage is toggle dictation: `dictate toggle` starts it and a second `dictate toggle`
//! stops it, and 3 minutes of scripted speech with pauses reach the pane in pieces. The fourth
//! stage does the same with Parakeet and Silero VAD, when both model files are there
//! (`AGENTSI_STT_DIR` and `AGENTSI_VAD_DIR`, else the standard folders).
//! The microphone is never opened.

use agentsi_core::server::{bind, euid};
use agentsi_core::state::Core;
use agentsi_pill::dictate::{
    CoreDelivery, Dictator, Mic, OpenDetector, OpenRecognizer, Recognizer,
};
use agentsi_pill::link::{self, LinkEvent};
use agentsi_pill::state::{Model, Phase};
use agentsi_proto::client::Client;
use agentsi_proto::socket::SocketPath;
use agentsi_proto::terminal_api::{Create, CreateParams};
use agentsi_proto::voice_api::{FocusReport, FocusReportParams};
use agentsi_voice::stt::{self, Clip, Transcriber};
use agentsi_voice::vad::{self, Classifier};
use futures::channel::mpsc::unbounded;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const WAV: &str = "../agentsi-voice/tests/fixtures/agentsi-story.wav";
const SAID: &str = "Please open AgentSI and show me the second story.";

struct WavMic(Clip);

impl Mic for WavMic {
    fn start(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn finish(&mut self) -> Result<Clip, String> {
        Ok(Clip {
            samples: self.0.samples.clone(),
            rate: self.0.rate,
        })
    }
    fn take(&mut self) -> Result<Clip, String> {
        Err("hold dictation does not take".into())
    }
    fn cancel(&mut self) {}
}

struct Said(&'static str);

impl Recognizer for Said {
    fn transcribe(&mut self, samples: &[f32]) -> Result<String, String> {
        assert!(
            samples.len() > 16_000,
            "the clip reaches the model at 16 kHz"
        );
        Ok(self.0.to_owned())
    }
}

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

/// The pill without its window, and the pane that it types into.
struct Rig {
    out: PathBuf,
    phases: Arc<Mutex<Vec<Phase>>>,
    _dictator: Arc<Dictator>,
    tx: futures::channel::mpsc::UnboundedSender<LinkEvent>,
    stopped: Arc<std::sync::atomic::AtomicBool>,
}

impl Rig {
    /// Ends the window thread, so the next stage has the only pill that answers.
    fn close(self) {
        self.stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = self.tx.unbounded_send(LinkEvent::DictationTarget(None));
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

/// A pane that writes the exact bytes it gets to `out`.
fn start_pane(socket: &SocketPath, dir: &Path, out: &Path) -> String {
    let mut client = Client::connect(&socket.path).unwrap();
    let info = client
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
        .unwrap();
    client
        .call::<FocusReport>(&FocusReportParams {
            terminal: Some(info.id.clone()),
        })
        .unwrap();
    info.id
}

/// The link, the state model and the dictator, as the window runs them.
fn start_pill(clip: Clip, open_recognizer: OpenRecognizer, out: PathBuf) -> Rig {
    let open_mic: agentsi_pill::dictate::OpenMic = Box::new(move || {
        Ok(Box::new(WavMic(Clip {
            samples: clip.samples.clone(),
            rate: clip.rate,
        })) as Box<dyn Mic>)
    });
    start_pill_with(
        open_mic,
        open_recognizer,
        no_detector(),
        Duration::from_millis(50),
        out,
    )
}

fn start_pill_with(
    open_mic: agentsi_pill::dictate::OpenMic,
    open_recognizer: OpenRecognizer,
    open_detector: OpenDetector,
    poll: Duration,
    out: PathBuf,
) -> Rig {
    let (tx, mut rx) = unbounded::<LinkEvent>();
    link::spawn(tx.clone());
    let dictator = Arc::new(Dictator::start_polling(
        poll,
        open_mic,
        open_recognizer,
        open_detector,
        Box::new(CoreDelivery),
        tx.clone(),
        true,
    ));
    // A press while the model loads is refused, so the test waits for the model.
    assert!(dictator.wait_until_loaded(Duration::from_secs(120)));
    let phases = Arc::new(Mutex::new(vec![Phase::Hidden]));
    let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (effects, seen_by_thread, stop) = (dictator.clone(), phases.clone(), stopped.clone());
    std::thread::spawn(move || {
        let mut model = Model::default();
        while let Some(event) = futures::executor::block_on(futures::StreamExt::next(&mut rx)) {
            if stop.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            if let LinkEvent::Input(input) = event {
                for effect in model.apply(input) {
                    effects.effect(&effect);
                }
                let mut seen = seen_by_thread.lock().unwrap();
                if seen.last() != Some(&model.phase()) {
                    seen.push(model.phase());
                }
            }
        }
    });
    Rig {
        out,
        phases,
        _dictator: dictator,
        tx,
        stopped,
    }
}

fn dictate(args: &[&str]) -> ExitCode {
    agentsi_core::dictate::run(args)
}

/// Holds the key for a moment, releases it, and returns the time from the release
/// to the exact bytes in the pane.
///
/// `cat` keeps the file open, so the file holds each dictation after the last: `times` of them.
fn hold_and_release(rig: &Rig, expect: &str, times: usize) -> Duration {
    let expect = expect.repeat(times);
    let expect = expect.as_str();
    // A press can come before this pill subscribes, and the core then sends it to nobody.
    // A second press does nothing while the pill listens, so the test presses until it listens.
    wait_for("the pill shows listening", || {
        let _ = dictate(&["start"]);
        std::thread::sleep(Duration::from_millis(100));
        (rig.phases.lock().unwrap().last() == Some(&Phase::Listening)).then_some(())
    });
    std::thread::sleep(Duration::from_millis(100));
    let released = Instant::now();
    assert_eq!(dictate(&["stop"]), ExitCode::SUCCESS);
    wait_for("the text is in the pane", || {
        (std::fs::read_to_string(&rig.out).ok()? == expect).then_some(())
    });
    let took = released.elapsed();
    // No Return follows: the pane gets nothing more.
    std::thread::sleep(Duration::from_millis(400));
    let bytes = std::fs::read(&rig.out).unwrap();
    assert_eq!(bytes, expect.as_bytes(), "the pane got more than the text");
    assert!(!bytes.contains(&b'\r') && !bytes.contains(&b'\n'));
    wait_for("the pill hides", || {
        (rig.phases.lock().unwrap().last() == Some(&Phase::Hidden)).then_some(())
    });
    took
}

/// Times the release to the first new byte in the pane, for a clip of any text.
fn release_to_text(rig: &Rig) -> Duration {
    let before = std::fs::metadata(&rig.out).map_or(0, |m| m.len());
    wait_for("the pill shows listening", || {
        let _ = dictate(&["start"]);
        std::thread::sleep(Duration::from_millis(100));
        (rig.phases.lock().unwrap().last() == Some(&Phase::Listening)).then_some(())
    });
    let released = Instant::now();
    assert_eq!(dictate(&["stop"]), ExitCode::SUCCESS);
    wait_for("new text in the pane", || {
        (std::fs::metadata(&rig.out).ok()?.len() > before).then_some(())
    });
    let took = released.elapsed();
    wait_for("the pill hides", || {
        (rig.phases.lock().unwrap().last() == Some(&Phase::Hidden)).then_some(())
    });
    took
}

/// `AGENTSI_DICTATE_BENCH` lists WAV files, separated by `:`. Each one is dictated 5 times
/// after a warm-up, and the test prints the delay from the release to the text.
fn bench(socket: &SocketPath, dir: &Path, model: &Path, files: &str) {
    for (n, file) in files.split(':').filter(|f| !f.is_empty()).enumerate() {
        let clip = agentsi_voice::stt::read_wav(Path::new(file)).unwrap();
        let seconds = clip.seconds();
        let out = dir.join(format!("bench{n}.bytes"));
        start_pane(socket, dir, &out);
        let model = model.to_owned();
        let rig = start_pill(
            clip,
            Box::new(move || {
                Transcriber::open(&model)
                    .map(|t| Box::new(Parakeet(t)) as Box<dyn Recognizer>)
                    .map_err(|e| e.to_string())
            }),
            out,
        );
        release_to_text(&rig);
        let mut times: Vec<Duration> = (0..5).map(|_| release_to_text(&rig)).collect();
        times.sort();
        eprintln!(
            "[bench] {file}: {seconds:.1} s of speech, release to text min {:?}, median {:?}, max {:?}, load average {}",
            times[0],
            times[2],
            times[4],
            load_average()
        );
        rig.close();
    }
}

/// Read at run time, so a test binary built in one checkout still finds its files.
fn manifest_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"))
}

fn load_average() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .map(|s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

fn stage(
    socket: &SocketPath,
    dir: &Path,
    name: &str,
    open: OpenRecognizer,
    clip: Clip,
    expect: &str,
) {
    let out = dir.join(format!("{name}.bytes"));
    start_pane(socket, dir, &out);
    let rig = start_pill(clip, open, out);
    let first = hold_and_release(&rig, expect, 1);
    // The second dictation shows the delay with the model warm.
    let second = hold_and_release(&rig, expect, 2);
    let seen = rig.phases.lock().unwrap().clone();
    assert!(
        seen.windows(3)
            .any(|w| w == [Phase::Listening, Phase::Transcribing, Phase::Hidden]),
        "phases {seen:?}"
    );
    eprintln!(
        "[{name}] release to text: first {first:?}, second {second:?}, load average {}",
        load_average()
    );
    rig.close();
}

fn no_detector() -> OpenDetector {
    Box::new(|| Err("no voice detector in this stage".into()))
}

/// A microphone that plays a script, `chunk` samples at each take.
struct ScriptMic {
    audio: Vec<f32>,
    at: usize,
    chunk: usize,
}

impl Mic for ScriptMic {
    fn start(&mut self) -> Result<(), String> {
        self.at = 0;
        Ok(())
    }
    fn take(&mut self) -> Result<Clip, String> {
        let end = (self.at + self.chunk).min(self.audio.len());
        let samples = self.audio[self.at..end].to_vec();
        self.at = end;
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

/// Names each piece "p1", "p2", and keeps the length of each in samples.
struct Counting {
    lengths: Arc<Mutex<Vec<usize>>>,
}

impl Recognizer for Counting {
    fn transcribe(&mut self, samples: &[f32]) -> Result<String, String> {
        let mut lengths = self.lengths.lock().unwrap();
        lengths.push(samples.len());
        Ok(format!("p{}", lengths.len()))
    }
}

/// Toggle dictation with Parakeet and Silero VAD: the clip three times, with a pause between.
fn real_toggle_stage(socket: &SocketPath, dir: &Path, model: PathBuf, vad_dir: PathBuf) {
    let out = dir.join("real-toggle.bytes");
    start_pane(socket, dir, &out);
    let wav = agentsi_voice::stt::read_wav(&manifest_dir().join(WAV)).unwrap();
    let mut audio = Vec::new();
    for _ in 0..3 {
        audio.extend(&wav.samples);
        audio.extend(vec![0.0; 16_000]);
    }
    let rig = start_pill_with(
        Box::new(move || {
            Ok(Box::new(ScriptMic {
                audio: audio.clone(),
                at: 0,
                chunk: 1_600,
            }) as Box<dyn Mic>)
        }),
        Box::new(move || {
            Transcriber::open(&model)
                .map(|t| Box::new(Parakeet(t)) as Box<dyn Recognizer>)
                .map_err(|e| e.to_string())
        }),
        Box::new(move || {
            vad::Silero::open(&vad_dir)
                .map(|d| Box::new(d) as Box<dyn Classifier + Send>)
                .map_err(|e| e.to_string())
        }),
        Duration::from_millis(1),
        out.clone(),
    );
    wait_for("the pill shows listening", || {
        let _ = dictate(&["start"]);
        std::thread::sleep(Duration::from_millis(100));
        (rig.phases.lock().unwrap().last() == Some(&Phase::Listening)).then_some(())
    });
    let expect = format!("{SAID} {SAID} {SAID}");
    wait_for("the three sentences are in the pane", || {
        (std::fs::read_to_string(&out).ok()? == expect).then_some(())
    });
    assert_eq!(rig.phases.lock().unwrap().last(), Some(&Phase::Listening));
    assert_eq!(dictate(&["toggle"]), ExitCode::SUCCESS);
    wait_for("the pill hides", || {
        (rig.phases.lock().unwrap().last() == Some(&Phase::Hidden)).then_some(())
    });
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(std::fs::read_to_string(&out).unwrap(), expect);
    rig.close();
}

fn press_toggle_until(rig: &Rig, phase: Phase) {
    assert_eq!(dictate(&["toggle"]), ExitCode::SUCCESS);
    wait_for("the phase changes", || {
        (rig.phases.lock().unwrap().last() == Some(&phase)).then_some(())
    });
}

/// Toggle dictation through a real core: 3 minutes of scripted speech with pauses.
fn toggle_stage(socket: &SocketPath, dir: &Path) {
    let out = dir.join("toggle.bytes");
    start_pane(socket, dir, &out);
    // Phrases of 7 to 40 s with a pause of 0.9 s. The longest phrases pass the 30 s limit.
    let mut audio = Vec::new();
    let mut seconds = 0.0;
    let mut n = 0;
    while seconds < 180.0 {
        let length = 7.0 + (n * 13 % 34) as f32;
        audio
            .extend((0..(length * 16_000.0) as usize).map(|i| if i % 2 == 0 { 0.5 } else { -0.5 }));
        audio.extend(vec![0.0; 14_400]);
        seconds += length + 0.9;
        n += 1;
    }
    let lengths = Arc::new(Mutex::new(Vec::new()));
    let counted = lengths.clone();
    let rig = start_pill_with(
        Box::new(move || {
            Ok(Box::new(ScriptMic {
                audio: audio.clone(),
                at: 0,
                chunk: 8_000,
            }) as Box<dyn Mic>)
        }),
        Box::new(move || {
            Ok(Box::new(Counting {
                lengths: counted.clone(),
            }) as Box<dyn Recognizer>)
        }),
        Box::new(|| Ok(Box::new(Loud) as Box<dyn Classifier + Send>)),
        Duration::from_millis(1),
        out.clone(),
    );
    // The earlier pill may still hold the subscription for one more event, and the new link
    // may not be subscribed yet: the press that starts is repeated until the pill listens.
    wait_for("the pill shows listening", || {
        let _ = dictate(&["start"]);
        std::thread::sleep(Duration::from_millis(100));
        (rig.phases.lock().unwrap().last() == Some(&Phase::Listening)).then_some(())
    });
    // The text arrives while the pill listens: the stop comes after all of it.
    let pieces = wait_for("all the pieces are in the pane", || {
        let count = lengths.lock().unwrap().len();
        (count >= 6 && std::fs::read_to_string(&out).ok()?.split(' ').count() == count)
            .then_some(count)
            .filter(|_| {
                // The script is over when the take returns nothing: the last piece closed.
                std::thread::sleep(Duration::from_millis(300));
                lengths.lock().unwrap().len() == count
            })
    });
    assert_eq!(
        rig.phases.lock().unwrap().last(),
        Some(&Phase::Listening),
        "the pill still listens: nothing stopped it"
    );
    let expect = (1..=pieces)
        .map(|i| format!("p{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(std::fs::read_to_string(&out).unwrap(), expect);
    let lengths = lengths.lock().unwrap().clone();
    assert!(
        lengths.iter().all(|l| *l <= 30 * 16_000),
        "a piece over 30 s: {lengths:?}"
    );
    // The second press stops it, and nothing more is typed.
    assert_eq!(dictate(&["toggle"]), ExitCode::SUCCESS);
    wait_for("the pill hides", || {
        (rig.phases.lock().unwrap().last() == Some(&Phase::Hidden)).then_some(())
    });
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(std::fs::read_to_string(&out).unwrap(), expect);
    // A press again starts a new dictation, and the next press stops it, with no more start.
    press_toggle_until(&rig, Phase::Listening);
    press_toggle_until(&rig, Phase::Hidden);
    eprintln!(
        "[toggle] {pieces} pieces, longest {} s",
        lengths.iter().max().unwrap() / 16_000
    );
    rig.close();
}

#[test]
fn a_wav_dictated_into_a_pane_arrives_as_text_with_no_return() {
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

    let wav = agentsi_voice::stt::read_wav(&manifest_dir().join(WAV)).unwrap();
    assert_eq!(wav.rate, 16_000);
    let clip = || Clip {
        samples: wav.samples.clone(),
        rate: wav.rate,
    };

    // Stage 1: a fake model that mis-hears the product name. The vocabulary fixes it, and
    // the control characters of a bad transcript never reach the pane.
    stage(
        &socket,
        root,
        "fake",
        Box::new(|| {
            Ok(Box::new(Said(
                "Please open Agent C\nand show me\x1b[2Jthe second story.",
            )) as Box<dyn Recognizer>)
        }),
        clip(),
        "Please open AgentSI and show me [2Jthe second story.",
    );

    // Stage 2: Parakeet, when the model files are there.
    let dir_of_model = std::env::var_os("AGENTSI_STT_DIR")
        .map(PathBuf::from)
        .or_else(stt::model_dir);
    match dir_of_model.filter(|d| d.join("nemo128.onnx").is_file()) {
        Some(model) => {
            let again = model.clone();
            stage(
                &socket,
                root,
                "parakeet",
                Box::new(move || {
                    Transcriber::open(&model)
                        .map(|t| Box::new(Parakeet(t)) as Box<dyn Recognizer>)
                        .map_err(|e| e.to_string())
                }),
                clip(),
                SAID,
            );
            if let Ok(files) = std::env::var("AGENTSI_DICTATE_BENCH") {
                bench(&socket, root, &again, &files);
            }
        }
        None => eprintln!("[parakeet] skipped: the model files are not downloaded"),
    }

    // Stage 3 and 4: toggle dictation. The key file names the mode, as the app writes it.
    let keys = root.join("config/agentsi/keys.toml");
    std::fs::create_dir_all(keys.parent().unwrap()).unwrap();
    std::fs::write(
        &keys,
        "[dictation]\nkey = \"SUPER + F10\"\nmode = \"toggle\"\n",
    )
    .unwrap();
    toggle_stage(&socket, root);
    let stt_dir = std::env::var_os("AGENTSI_STT_DIR")
        .map(PathBuf::from)
        .or_else(stt::model_dir)
        .filter(|d| d.join("nemo128.onnx").is_file());
    let vad_dir = std::env::var_os("AGENTSI_VAD_DIR")
        .map(PathBuf::from)
        .or_else(vad::model_dir)
        .filter(|d| d.join(vad::FILE).is_file());
    match (stt_dir, vad_dir) {
        (Some(stt_dir), Some(vad_dir)) => real_toggle_stage(&socket, root, stt_dir, vad_dir),
        _ => eprintln!(
            "[real toggle] skipped: the speech-in or the voice detector files are missing"
        ),
    }
    std::fs::remove_file(&keys).unwrap();
}

struct Parakeet(Transcriber);

impl Recognizer for Parakeet {
    fn transcribe(&mut self, samples: &[f32]) -> Result<String, String> {
        self.0.transcribe(samples).map_err(|e| e.to_string())
    }
}
