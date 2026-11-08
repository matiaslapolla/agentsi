//! Calls (U26): the audio of a meeting. It ports the Python package `agentsi-voice`
//! (`render`) and the listened mark of the web app. `codec` reads and writes the
//! audio files, `files` finds them, `render` makes a call from a script, and
//! `view` answers the call screen.
//!
//! The speech engine is behind the trait [`Synth`], so this module links no
//! ONNX Runtime. The core gives it Kokoro.

pub mod codec;
mod files;
mod render;
mod view;

pub use files::{AudioFile, Format, audio_dir, find_audio, read_timings, safe_segment};
pub use render::{Progress, Rendered, ScriptItem, Stage, Synth, render, script_of};
pub use view::{CallView, audio_chunk, get, set_heard};

use std::fmt;

#[derive(Debug)]
pub enum Error {
    NotFound(String),
    /// The minutes have no script, or the script is malformed.
    Invalid(String),
    Unavailable(String),
    /// The owner canceled the render. It wrote nothing.
    Canceled,
    Failed(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound(m) | Error::Invalid(m) | Error::Unavailable(m) | Error::Failed(m) => {
                f.write_str(m)
            }
            Error::Canceled => f.write_str("The render was canceled."),
        }
    }
}

impl std::error::Error for Error {}

impl From<crate::Error> for Error {
    fn from(e: crate::Error) -> Error {
        match e {
            crate::Error::NotFound(m) => Error::NotFound(m),
            crate::Error::Unavailable(m) => Error::Unavailable(m),
        }
    }
}

impl From<agentsi_store::Error> for Error {
    fn from(e: agentsi_store::Error) -> Error {
        Error::Unavailable(e.to_string())
    }
}

impl From<codec::Error> for Error {
    fn from(e: codec::Error) -> Error {
        Error::Failed(e.0)
    }
}
