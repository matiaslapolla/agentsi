//! Speech in: Parakeet TDT 0.6B v3 (int8) on ONNX Runtime.
//!
//! [`Transcriber`] turns 16 kHz mono samples into text. The audio features come from
//! `nemo128.onnx` of the catalog, so no feature code lives here. The steps follow
//! onnx-asr 0.12.0 (MIT), which the P2 harness measured.

pub mod audio;
pub mod decode;
mod parakeet;

pub use audio::{Clip, RATE, Resampler, read_wav, resample};
pub use parakeet::Transcriber;

use std::fmt;
use std::path::PathBuf;

/// The catalog id of the model.
pub const MODEL_ID: &str = "parakeet-tdt-v3-int8";
/// The files of the catalog entry that the model reads.
pub const FILES: [&str; 4] = [
    parakeet::PREPROCESSOR,
    parakeet::ENCODER,
    parakeet::DECODER,
    parakeet::VOCAB,
];
/// The longest clip that one call accepts. A longer clip needs a split first.
pub const MAX_SECONDS: f32 = 120.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A model file is missing. The settings screen downloads it.
    NotDownloaded(PathBuf),
    Model(String),
    Audio(String),
    Runtime(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotDownloaded(_) => write!(f, "The speech-in model is not downloaded"),
            Error::Model(e) => write!(f, "Parakeet: {e}"),
            Error::Audio(e) => write!(f, "{e}"),
            Error::Runtime(e) => write!(f, "Parakeet: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// The folder that holds the model files: `<models>/catalog/parakeet-tdt-v3-int8/`.
pub fn model_dir() -> Option<PathBuf> {
    agentsi_models::default_root().map(|root| root.join("catalog").join(MODEL_ID))
}
