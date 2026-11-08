//! Prints the Kokoro phonemes of each stdin line, as `text<TAB>phonemes`.
//! `printf 'Hello world.\n' | cargo run -p agentsi-voice --example phonemes [-- <prefix word>]`

use agentsi_voice::tts::{Espeak, G2p, phonemes, vocab};
use std::io::Read;

struct Prefixed(Espeak, String);

impl G2p for Prefixed {
    fn ipa(&self, piece: &str) -> Result<String, String> {
        let head = self.0.ipa(&self.1)?;
        let head = head.trim();
        let out = self.0.ipa(&format!("{} {piece}", self.1))?;
        let out = out.trim().replace('\n', " ");
        out.strip_prefix(head)
            .map(str::to_owned)
            .ok_or(format!("prefix lost: {out}"))
    }
}

fn main() {
    let espeak = Espeak::find("en-us").expect("espeak-ng is on the PATH");
    let g2p: Box<dyn G2p> = match std::env::args().nth(1) {
        Some(word) => Box::new(Prefixed(espeak, word)),
        None => Box::new(espeak),
    };
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .expect("stdin is UTF-8");
    for line in text.split('\n').filter(|l| !l.trim().is_empty()) {
        match phonemes::phonemize(line, g2p.as_ref()) {
            Ok(raw) => println!("{line}\t{}", vocab::clean(&raw)),
            Err(e) => println!("{line}\tERROR {e}"),
        }
    }
}
