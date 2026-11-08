//! Speech in against the real model. The tests that need the model files skip with a
//! message when they are missing: set `AGENTSI_STT_DIR` to a folder that holds them, or
//! download the model in the settings screen.
//!
//! The WAV is synthetic speech (a Kokoro voice, not a person):
//! "Please open AgentSI and show me the second story."

use agentsi_voice::stt::{self, Error, Transcriber, read_wav};
use agentsi_voice::vocabulary::Vocabulary;
use std::path::PathBuf;

fn model_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("AGENTSI_STT_DIR")
        .map(PathBuf::from)
        .or_else(stt::model_dir)?;
    dir.join("nemo128.onnx").is_file().then_some(dir)
}

/// Read at run time, so a test binary built in one checkout still finds its files.
fn manifest_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"))
}

fn fixture() -> PathBuf {
    manifest_dir().join("tests/fixtures/agentsi-story.wav")
}

#[test]
fn a_folder_with_no_model_is_not_downloaded() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        Transcriber::open(dir.path()),
        Err(Error::NotDownloaded(_))
    ));
}

#[test]
fn a_broken_vocabulary_file_is_a_model_error() {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "nemo128.onnx",
        "encoder-model.int8.onnx",
        "decoder_joint-model.int8.onnx",
    ] {
        std::fs::write(dir.path().join(name), b"x").unwrap();
    }
    std::fs::write(dir.path().join("vocab.txt"), "no blank here 1\n").unwrap();
    assert!(matches!(
        Transcriber::open(dir.path()),
        Err(Error::Model(_))
    ));
}

#[test]
fn parakeet_hears_the_wav_and_the_vocabulary_fixes_the_name() {
    let Some(dir) = model_dir() else {
        eprintln!("skipped: the speech-in model files are not downloaded");
        return;
    };
    let mut model = Transcriber::open(&dir).expect("the model opens");
    let clip = read_wav(&fixture()).unwrap();
    let text = model.transcribe(&clip.at_model_rate()).unwrap();
    let heard = text.to_lowercase();
    assert!(
        heard.contains("open") && heard.contains("second story"),
        "heard {text:?}"
    );
    let fixed = Vocabulary::seed(&["AgentSI"]).apply(&text);
    assert!(fixed.contains("AgentSI"), "heard {text:?}, fixed {fixed:?}");

    // A 48 kHz copy of the clip gives the same words: the resampler works.
    let up = stt::resample(&clip.samples, clip.rate, 48_000);
    let again = stt::Clip {
        samples: up,
        rate: 48_000,
    };
    let text48 = model.transcribe(&again.at_model_rate()).unwrap();
    assert_eq!(
        Vocabulary::seed(&["AgentSI"]).apply(&text48),
        fixed,
        "the 48 kHz copy was heard as {text48:?}"
    );
}

#[test]
fn silence_and_short_clips_give_no_text_and_a_long_clip_is_refused() {
    let Some(dir) = model_dir() else {
        eprintln!("skipped: the speech-in model files are not downloaded");
        return;
    };
    let mut model = Transcriber::open(&dir).expect("the model opens");
    assert_eq!(model.transcribe(&[]).unwrap(), "");
    assert_eq!(model.transcribe(&[0.0; 800]).unwrap(), "");
    assert_eq!(model.transcribe(&vec![0.0; 32_000]).unwrap(), "");
    let long = vec![0.0; (stt::MAX_SECONDS as usize + 1) * 16_000];
    assert!(matches!(model.transcribe(&long), Err(Error::Audio(_))));
}

#[test]
fn the_catalog_entry_holds_each_file_that_the_model_reads() {
    let catalog = agentsi_models::Catalog::builtin();
    let entry = catalog
        .get(stt::MODEL_ID)
        .expect("the entry is in the catalog");
    assert_eq!(entry.kind, agentsi_models::Kind::SpeechIn);
    assert_eq!(entry.license, "CC-BY-4.0");
    for file in stt::FILES {
        assert!(
            entry.files.iter().any(|f| f.name == file),
            "the catalog entry has no {file}"
        );
    }
    // Each file is pinned: a size and a SHA-256.
    assert!(
        entry
            .files
            .iter()
            .all(|f| f.size > 0 && f.sha256.len() == 64)
    );
}

/// ONNX Runtime allows one environment for each process, and the pill loads the voice
/// and the speech-in model together. Two models in one process must both open.
#[test]
fn the_speech_in_model_and_the_voice_open_in_one_process() {
    let Some(dir) = model_dir() else {
        eprintln!("skipped: the speech-in model files are not downloaded");
        return;
    };
    let _first = Transcriber::open(&dir).expect("the first model opens");
    let _second = Transcriber::open(&dir).expect("a second model opens in the same process");
    let Some(kokoro) = agentsi_models::default_root().map(|r| r.join("catalog/kokoro")) else {
        return;
    };
    let (model, voices) = (
        kokoro.join("kokoro-v1.0.onnx"),
        kokoro.join("voices-v1.0.bin"),
    );
    if model.is_file() && voices.is_file() {
        agentsi_voice::tts::Kokoro::open(&model, &voices)
            .expect("the voice opens next to the model");
    } else {
        eprintln!("the voice half of this test skipped: Kokoro is not downloaded");
    }
}
