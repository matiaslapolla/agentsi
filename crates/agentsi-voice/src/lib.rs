//! Speech in and out on ONNX Runtime.
//!
//! - [`text`]: the text rules of voice mode, what part of a reply the pill speaks,
//!   and the transcript of a turn. They port `plugin/hooks/voice-mode.mjs`.
//! - [`tts`]: Kokoro on ONNX Runtime, with phonemes from the `espeak-ng` program.
//! - [`stt`]: Parakeet TDT on ONNX Runtime, from 16 kHz samples to text.
//! - [`vad`]: Silero VAD on ONNX Runtime, and the cutter that splits speech at the pauses.
//! - [`vocabulary`]: the owner's list of terms that the transcript is corrected with.

#[cfg(any(feature = "tts", feature = "stt"))]
mod runtime;
#[cfg(feature = "stt")]
pub mod stt;
pub mod text;
#[cfg(feature = "tts")]
pub mod tts;
pub mod vad;
pub mod vocabulary;

pub use text::{RULES, SCREEN_LINE, VOICE, spoken_sentences, transcript_entry, transcript_head};
