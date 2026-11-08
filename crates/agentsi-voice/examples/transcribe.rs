//! Transcribes WAV files with Parakeet and prints one JSON line for each file.
//! `cargo run --release -p agentsi-voice --example transcribe -- [--dir <model dir>] [--vocabulary] <a.wav> ...`
//!
//! The model folder is `<models>/catalog/parakeet-tdt-v3-int8/` unless `--dir` names another.
//! `--vocabulary` applies `<config>/agentsi/vocabulary.toml`.

use agentsi_voice::stt::{self, Transcriber, audio};
use agentsi_voice::vocabulary::{self, Vocabulary};
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let (mut dir, mut vocab, mut files) = (None, false, Vec::new());
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dir" => dir = args.next().map(PathBuf::from),
            "--vocabulary" => vocab = true,
            _ => files.push(PathBuf::from(a)),
        }
    }
    if files.is_empty() {
        eprintln!("usage: transcribe [--dir <model dir>] [--vocabulary] <a.wav> ...");
        std::process::exit(2);
    }
    let dir = dir.or_else(stt::model_dir).expect("a model folder");
    let words = if vocab {
        vocabulary::path()
            .and_then(|p| Vocabulary::load(&p).ok().flatten())
            .unwrap_or_default()
    } else {
        Vocabulary::default()
    };
    let started = Instant::now();
    let mut model = Transcriber::open(&dir).unwrap_or_else(|e| {
        eprintln!("transcribe: {e}");
        std::process::exit(1)
    });
    eprintln!("open: {:?}", started.elapsed());
    for file in files {
        let clip = audio::read_wav(&file).expect("a WAV file");
        let t = Instant::now();
        let samples = clip.at_model_rate();
        let resample = t.elapsed();
        let text = model.transcribe(&samples).expect("a transcript");
        println!(
            "{}",
            serde_json::json!({
                "file": file.file_name().map(|n| n.to_string_lossy().into_owned()),
                "audio_s": clip.seconds(),
                "resample_s": resample.as_secs_f32(),
                "total_s": t.elapsed().as_secs_f32(),
                "text": words.apply(&text),
            })
        );
    }
}
