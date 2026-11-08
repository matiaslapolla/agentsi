//! Speech out with a fake phonemizer program, and with the real `espeak-ng` and model when
//! they exist. A test that needs a missing program or model says so and passes.

use agentsi_voice::tts::{Espeak, G2p, Options, Voice, phonemes, vocab};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

static SPAWNS: Mutex<()> = Mutex::new(());

/// Runs the tests of this file one at a time. A test that writes a script holds the file open
/// for write. If another test thread forks at that moment, the child keeps the descriptor until
/// its exec, and running the script fails with `Text file busy`.
fn serial() -> MutexGuard<'static, ()> {
    SPAWNS.lock().unwrap_or_else(|e| e.into_inner())
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("agentsi-tts-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A program that answers like espeak-ng: `wˈɜːd` for each input word, and `zˈiːbɹə` for "zebra".
fn fake_espeak(dir: &Path) -> PathBuf {
    let path = dir.join("fake-espeak");
    std::fs::write(
        &path,
        "#!/bin/sh\nread -r line\nout=''\nfor w in $line; do\n  if [ \"$w\" = zebra ]; then out=\"$out zˈiːbɹə\"; else out=\"$out wˈɜːd\"; fi\ndone\necho \"$out\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn real_espeak() -> Option<Espeak> {
    let found = Espeak::find("en-us");
    if found.is_none() {
        eprintln!("skipped: espeak-ng is not on the PATH");
    }
    found
}

#[test]
fn the_fake_program_phonemizes_each_piece() {
    let _serial = serial();
    let dir = temp("fake");
    let espeak = Espeak::with_program(fake_espeak(&dir), "en-us");
    let raw = phonemes::phonemize("Hello there, world.", &espeak).unwrap();
    assert_eq!(vocab::clean(&raw), "wˈɜːd wˈɜːd, wˈɜːd.");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_sentence_with_many_marks_keeps_its_order() {
    let _serial = serial();
    let dir = temp("many");
    let espeak = Espeak::with_program(fake_espeak(&dir), "en-us");
    let text = vec!["word"; 40].join(", ");
    let raw = phonemes::phonemize(&text, &espeak).unwrap();
    assert_eq!(vocab::clean(&raw), vec!["wˈɜːd"; 40].join(", "));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_failing_program_is_an_error() {
    let _serial = serial();
    let dir = temp("fail");
    let path = dir.join("fail");
    std::fs::write(&path, "#!/bin/sh\necho broken >&2\nexit 3\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let e = Espeak::with_program(&path, "en-us").ipa("x").unwrap_err();
    assert!(e.contains("broken"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The phonemes of kokoro-onnx 0.6.1 for these lines, from `packages/voice/.venv`, recorded 2026-11-03.
const KOKORO_ONNX: &[(&str, &str)] = &[
    (
        "Hello world, this is a test. And another one",
        "həlˈoʊ wˈɜːld, ðɪs ɪz ɐ tˈɛst. ænd ɐnˈʌðɚ wˌʌn",
    ),
    (
        "Wait... \"quoted\" (aside) — dash; colon: end!",
        "wˈeɪt... \"kwˈoʊɾᵻd\" (ɐsˈaɪd) — dˈæʃ; kˈoʊlən: ˈɛnd!",
    ),
    ("Pi is 3.14 and done.", "pˈaɪ ɪz θɹˈiː. fˈɔːɹtiːn ænd dˈʌn"),
    ("A list: one; two; three.", "ɐ lˈɪst: wˌʌn; tˈuː; θɹˈiː."),
    (
        "I'm, you're, we've, they'd, it'll.",
        "aɪm, jʊɹ, wiːv, ðeɪd, ˌɪɾəl.",
    ),
];

#[test]
fn espeak_ng_gives_the_phonemes_of_kokoro_onnx() {
    let _serial = serial();
    let Some(espeak) = real_espeak() else { return };
    for (text, want) in KOKORO_ONNX {
        let raw = phonemes::phonemize(text, &espeak).unwrap();
        assert_eq!(&vocab::clean(&raw), want, "{text}");
    }
}

#[test]
fn the_real_voice_speaks_one_sentence() {
    let _serial = serial();
    let Some(espeak) = real_espeak() else { return };
    let options = Options {
        models: None,
        data_root: None,
        g2p: Some(Box::new(espeak)),
    };
    let mut voice = match Voice::open(options) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("skipped: {e}");
            return;
        }
    };
    let (name, warnings) = voice.voice_for("pm");
    assert_eq!((name.as_str(), warnings.len()), ("af_heart", 0));
    let sentences = voice.sentences("The QA run passed.\nIt is green");
    assert_eq!(sentences, vec!["The Q A run passed.", "It is green"]);
    let samples = voice.render(&name, &sentences[0]).unwrap();
    let seconds = samples.len() as f32 / agentsi_voice::tts::RATE as f32;
    assert!((0.5..4.0).contains(&seconds), "{seconds} s");
    assert!(samples.iter().any(|s| s.abs() > 0.05));
}
