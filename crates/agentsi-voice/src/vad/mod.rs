//! The voice detector: Silero VAD v5 on ONNX Runtime, and the cutter that splits a stream of
//! 16 kHz audio into pieces at the pauses.
//!
//! - [`Classifier`] marks one frame of 512 samples (32 ms) with a chance of speech.
//! - [`Silero`] is the classifier of the catalog entry `silero-vad-v5`.
//! - [`Cutter`] uses a classifier and cuts the speech at each pause. Toggle dictation uses a
//!   pause of 0.6 s. The pure part has no model, so a test feeds it a fake classifier.

mod cutter;
#[cfg(feature = "stt")]
mod silero;

pub use cutter::{Cutter, MAX_PIECE};
#[cfg(feature = "stt")]
pub use silero::{Silero, model_dir};

use std::fmt;
use std::path::PathBuf;

/// The sample rate of the detector.
pub const RATE: u32 = 16_000;
/// Samples in one frame: 32 ms.
pub const FRAME: usize = 512;
/// A frame above this chance counts as speech.
pub const THRESHOLD: f32 = 0.5;
/// The catalog id of the model.
pub const MODEL_ID: &str = "silero-vad-v5";
/// The file of the catalog entry that the model reads.
pub const FILE: &str = "silero-vad.onnx";

/// Frames in a span of seconds, rounded up.
pub fn frames_in(seconds: f32) -> usize {
    (seconds * RATE as f32 / FRAME as f32).ceil() as usize
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The model file is missing. The settings screen downloads it.
    NotDownloaded(PathBuf),
    Model(String),
    Runtime(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotDownloaded(_) => write!(f, "The voice detector is not downloaded"),
            Error::Model(e) => write!(f, "Silero VAD: {e}"),
            Error::Runtime(e) => write!(f, "Silero VAD: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// Marks frames of 16 kHz audio.
pub trait Classifier {
    /// The chance that `frame` holds speech, from 0 to 1. `frame` has [`FRAME`] samples, in
    /// the range -1 to 1. The classifier keeps the state of the stream between two calls.
    fn speech(&mut self, frame: &[f32]) -> Result<f32, Error>;
    /// Forgets the stream, so the next frame starts a new one.
    fn reset(&mut self);
}

impl<C: Classifier + ?Sized> Classifier for Box<C> {
    fn speech(&mut self, frame: &[f32]) -> Result<f32, Error> {
        (**self).speech(frame)
    }

    fn reset(&mut self) {
        (**self).reset()
    }
}
