//! The voice detector against the real model. The tests that need the model file skip with a
//! message when it is missing: set `AGENTSI_VAD_DIR` to a folder that holds `silero-vad.onnx`,
//! or download `silero-vad-v5` in the settings screen.
//!
//! The WAV is synthetic speech (a Kokoro voice, not a person), 3.3 s long.

use agentsi_voice::stt::read_wav;
use agentsi_voice::vad::{self, Classifier, Cutter, Error, FRAME, Silero, THRESHOLD};
use std::path::PathBuf;

fn model_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("AGENTSI_VAD_DIR")
        .map(PathBuf::from)
        .or_else(vad::model_dir)?;
    dir.join(vad::FILE).is_file().then_some(dir)
}

fn speech_clip() -> Vec<f32> {
    // Read at run time, so a test binary built in one checkout still finds its files.
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let clip = read_wav(&manifest.join("tests/fixtures/agentsi-story.wav")).unwrap();
    clip.at_model_rate()
}

/// The mark of each whole frame: true for speech.
fn marks(detector: &mut Silero, audio: &[f32]) -> Vec<bool> {
    audio
        .as_chunks::<FRAME>()
        .0
        .iter()
        .map(|f| detector.speech(f).unwrap() > THRESHOLD)
        .collect()
}

#[test]
fn a_folder_with_no_model_is_not_downloaded() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        Silero::open(dir.path()),
        Err(Error::NotDownloaded(_))
    ));
}

#[test]
fn the_detector_marks_speech_as_speech_and_silence_as_silence() {
    let Some(dir) = model_dir() else {
        eprintln!("skipped: the voice detector file is not downloaded");
        return;
    };
    let mut detector = Silero::open(&dir).expect("the model opens");
    let speech = speech_clip();

    let marked = marks(&mut detector, &speech);
    let share = marked.iter().filter(|m| **m).count() as f32 / marked.len() as f32;
    assert!(
        share > 0.6,
        "only {share:.2} of the speech clip is marked as speech"
    );

    // Zeros, and low noise at about -60 dB, are not speech.
    detector.reset();
    assert!(
        marks(&mut detector, &vec![0.0; 16_000 * 3])
            .iter()
            .all(|m| !m)
    );
    detector.reset();
    let noise: Vec<f32> = (0..16_000 * 3)
        .map(|i| 0.001 * ((i as f32 * 12.9898).sin() * 43_758.547).fract())
        .collect();
    assert!(marks(&mut detector, &noise).iter().all(|m| !m));

    // A pause inside a clip is marked as no speech, and the speech around it is not.
    detector.reset();
    let mut mixed = speech.clone();
    mixed.extend(vec![0.0; 16_000 * 2]);
    mixed.extend(&speech);
    let marked = marks(&mut detector, &mixed);
    let pause_from = speech.len() / FRAME + 4;
    let pause_to = (speech.len() + 16_000 * 2) / FRAME - 2;
    assert!(
        marked[pause_from..pause_to].iter().all(|m| !m),
        "{marked:?}"
    );
    assert!(marked[pause_to + 2..].iter().any(|m| *m));
}

#[test]
fn the_cutter_with_the_real_detector_cuts_three_minutes_in_ordered_pieces() {
    let Some(dir) = model_dir() else {
        eprintln!("skipped: the voice detector file is not downloaded");
        return;
    };
    let clip = speech_clip();
    // 3 min of the clip, with a pause of 0.9 s between two repeats and a long pause now and then.
    let mut audio = Vec::new();
    let mut repeats = 0;
    while audio.len() < 16_000 * 180 {
        audio.extend(&clip);
        let pause = if repeats % 5 == 4 { 4.0 } else { 0.9 };
        audio.extend(vec![0.0; (16_000.0 * pause) as usize]);
        repeats += 1;
    }
    let mut cutter = Cutter::new(Silero::open(&dir).unwrap(), 0.6);
    let mut pieces = Vec::new();
    for chunk in audio.chunks(800) {
        pieces.extend(cutter.feed(chunk).unwrap());
    }
    pieces.extend(cutter.finish());
    // One piece for each repeat: each pause is over 0.6 s and no repeat has one inside.
    assert_eq!(
        pieces.len(),
        repeats,
        "pieces of {:?} s",
        pieces.iter().map(|p| p.len() / 16_000).collect::<Vec<_>>()
    );
    for p in &pieces {
        let seconds = p.len() as f32 / 16_000.0;
        assert!((3.0..30.0).contains(&seconds), "{seconds}");
    }
}

#[test]
fn the_catalog_entry_pins_the_model_file() {
    let catalog = agentsi_models::Catalog::builtin();
    let entry = catalog
        .get(vad::MODEL_ID)
        .expect("the entry is in the catalog");
    assert_eq!(entry.license, "MIT");
    assert_eq!(entry.files.len(), 1);
    assert_eq!(entry.files[0].name, vad::FILE);
    assert!(entry.files[0].size > 1_000_000 && entry.files[0].sha256.len() == 64);
}
