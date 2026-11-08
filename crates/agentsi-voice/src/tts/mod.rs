//! Speech out: Kokoro v1.0 on ONNX Runtime, with phonemes from the `espeak-ng` program.
//!
//! [`Voice`] is the whole path from one line of a reply to 24 kHz samples:
//! the lexicon, the phonemes, the model and the level.

pub mod audio;
mod chunk;
pub mod kokoro;
pub mod phonemes;
pub mod vocab;
pub mod voices;

pub use audio::RATE;
pub use kokoro::Kokoro;
pub use phonemes::{Espeak, G2p};
pub use voices::{Lexicon, VoiceRef, Voices};

use std::fmt;
use std::path::PathBuf;

/// The silence between two sentences, as in `render.py`.
pub const SENTENCE_GAP: f32 = 0.2;
/// The catalog id of the model.
pub const MODEL_ID: &str = "kokoro";
const MODEL_FILE: &str = "kokoro-v1.0.onnx";
const VOICES_FILE: &str = "voices-v1.0.bin";
const WARM_UP: &str = "həlˈoʊ wˈɜːld.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A model file is missing. The settings screen downloads it.
    NotDownloaded(PathBuf),
    /// No `espeak-ng` program.
    NoEspeak,
    Phonemes(String),
    Voices(String),
    Config(String),
    Runtime(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotDownloaded(_) => write!(f, "The Kokoro voice is not downloaded"),
            Error::NoEspeak => write!(f, "espeak-ng is not installed"),
            Error::Phonemes(e) => write!(f, "espeak-ng: {e}"),
            Error::Voices(e) => write!(f, "Kokoro voices: {e}"),
            Error::Config(e) => write!(f, "{e}"),
            Error::Runtime(e) => write!(f, "Kokoro: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// Where the voice finds its files.
pub struct Options {
    /// `<models>/catalog/kokoro/` holds the model. `None` uses `agentsi_models::default_root`.
    pub models: Option<PathBuf>,
    /// The data root, for `voices.toml` and `lexicon.toml`. `None` uses the default files.
    pub data_root: Option<PathBuf>,
    /// The phonemizer. `None` finds `espeak-ng`.
    pub g2p: Option<Box<dyn G2p + Send>>,
}

impl Options {
    /// The default folders and the `espeak-ng` program.
    pub fn standard() -> Options {
        Options {
            models: None,
            data_root: voices::data_root(),
            g2p: None,
        }
    }
}

pub struct Voice {
    kokoro: Kokoro,
    g2p: Box<dyn G2p + Send>,
    voices: Voices,
    lexicon: Lexicon,
}

impl Voice {
    /// Loads the model and the settings, and runs the model once. It takes about two
    /// seconds, so the pill does it at start.
    pub fn open(options: Options) -> Result<Voice, Error> {
        let root = options.data_root.as_deref();
        let voices = Voices::load(root).map_err(Error::Config)?;
        let lexicon = Lexicon::load(root, &voices.lang).map_err(Error::Config)?;
        let g2p: Box<dyn G2p + Send> = match options.g2p {
            Some(g) => g,
            None => Box::new(Espeak::find(&voices.lang).ok_or(Error::NoEspeak)?),
        };
        let models = options
            .models
            .or_else(agentsi_models::default_root)
            .ok_or_else(|| Error::Config("HOME is not set".into()))?;
        let dir = models.join("catalog").join(MODEL_ID);
        let mut kokoro = Kokoro::open(&dir.join(MODEL_FILE), &dir.join(VOICES_FILE))?;
        // The first run of a session is about twice as slow as later runs.
        let warm = voices
            .kokoro(&voices.default.name, |n| kokoro.has_voice(n))
            .0;
        kokoro.speak(WARM_UP, &warm)?;
        Ok(Voice {
            kokoro,
            g2p,
            voices,
            lexicon,
        })
    }

    /// The sentences of a `voice.speak` text: one per line, split again at sentence
    /// ends, with the lexicon applied.
    pub fn sentences(&self, text: &str) -> Vec<String> {
        text.lines()
            .flat_map(|line| voices::sentences(line, &self.lexicon))
            .collect()
    }

    /// The Kokoro voice of `role`, and a warning for each fallback.
    pub fn voice_for(&self, role: &str) -> (String, Vec<String>) {
        self.voices.kokoro(role, |name| self.kokoro.has_voice(name))
    }

    /// The phonemes that Kokoro reads for one sentence.
    pub fn phonemes(&self, sentence: &str) -> Result<String, Error> {
        let raw = phonemes::phonemize(sentence, self.g2p.as_ref()).map_err(Error::Phonemes)?;
        Ok(vocab::clean(&raw))
    }

    /// One sentence as leveled samples at [`RATE`]. A sentence with no phonemes is empty.
    pub fn render(&mut self, voice: &str, sentence: &str) -> Result<Vec<f32>, Error> {
        let phonemes = self.phonemes(sentence)?;
        if phonemes.is_empty() {
            return Ok(Vec::new());
        }
        let mut samples = self.kokoro.speak(&phonemes, voice)?;
        audio::level(&mut samples);
        Ok(samples)
    }
}
