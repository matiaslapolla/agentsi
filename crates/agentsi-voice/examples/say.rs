//! Speaks a text into a WAV file with the role's voice, and prints the timings.
//! `cargo run --release -p agentsi-voice --example say -- <out.wav> <role> <text>`

use agentsi_voice::tts::{Options, SENTENCE_GAP, Voice, audio};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [out, role, text] = args.as_slice() else {
        eprintln!("usage: say <out.wav> <role> <text>");
        std::process::exit(2);
    };
    let started = Instant::now();
    let mut voice = Voice::open(Options::standard()).unwrap_or_else(|e| {
        eprintln!("say: {e}");
        std::process::exit(1)
    });
    eprintln!("open: {:?}", started.elapsed());
    let (name, warnings) = voice.voice_for(role);
    for w in warnings {
        eprintln!("warning: {w}");
    }
    let mut all = Vec::new();
    for sentence in voice.sentences(text) {
        let t = Instant::now();
        let phonemes = voice.phonemes(&sentence).expect("phonemes");
        let p = t.elapsed();
        let samples = voice.render(&name, &sentence).expect("render");
        eprintln!(
            "{:?} phonemes {p:?}, total {:?}, audio {:.2} s: {phonemes}",
            sentence,
            t.elapsed(),
            samples.len() as f32 / audio::RATE as f32
        );
        if !all.is_empty() {
            all.extend(audio::silence(SENTENCE_GAP));
        }
        all.extend(samples);
    }
    audio::write_wav(std::path::Path::new(out), &all).expect("the WAV file is written");
}
