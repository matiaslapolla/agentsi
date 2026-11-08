//! Kokoro and Parakeet in the process of the core. ONNX Runtime allows one environment for each
//! process, and both models take it from `agentsi_voice::runtime`. The call render of the core
//! opens Kokoro with `Voice::open(Options::standard())`. This test opens the same voice and the
//! speech-in model in one process, makes the voice say a sentence, and has Parakeet hear it.
//!
//! The test skips with a message when a model or `espeak-ng` is missing. Set `AGENTSI_STT_DIR`
//! for the speech-in model, and `XDG_DATA_HOME` for a models folder that holds Kokoro.

use agentsi_voice::stt::{self, Transcriber, audio};
use agentsi_voice::tts::{self, Options, Voice};
use std::path::PathBuf;

fn stt_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("AGENTSI_STT_DIR")
        .map(PathBuf::from)
        .or_else(stt::model_dir)?;
    dir.join("nemo128.onnx").is_file().then_some(dir)
}

#[test]
fn the_voice_and_the_speech_in_model_run_in_one_core_process() {
    let Some(dir) = stt_dir() else {
        eprintln!("skipped: the speech-in model is not downloaded");
        return;
    };
    // The voice opens first, as it does when a call renders before the owner dictates.
    let mut voice = match Voice::open(Options::standard()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("skipped: {e}");
            return;
        }
    };
    let mut ear = Transcriber::open(&dir).expect("the speech-in model opens next to the voice");
    let (name, _) = voice.voice_for("pm");
    let said = voice
        .render(&name, "Please open the second story.")
        .expect("the voice speaks");
    let heard = ear
        .transcribe(&audio::resample(&said, tts::RATE, audio::RATE))
        .expect("the model hears");
    assert!(
        heard.to_lowercase().contains("second story"),
        "the model heard: {heard}"
    );
    // A second voice render after the model ran still works: the environment is shared, not replaced.
    voice
        .render(&name, "Done.")
        .expect("the voice speaks again");
}
