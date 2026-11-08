use std::fmt;

/// A failure, in a person's words. `kind()` gives a stable tag for the settings screen.
#[derive(Debug)]
pub enum Error {
    /// The model id is not in the catalog.
    UnknownModel(String),
    /// Another download of this model holds the lock.
    Busy(String),
    /// The caller stopped the download. The `.partial` file stays.
    Canceled,
    /// The finished file has another hash than the catalog. The file is deleted.
    HashMismatch {
        file: String,
        expected: String,
        actual: String,
    },
    /// The network or the server failed after all retries.
    Network(String),
    /// The server sent bytes that cannot be right, such as more than the pinned size.
    BadResponse(String),
    /// A file with this name is in the model folder, and the catalog does not describe it
    /// (another size or hash, and no receipt). The file is left as it is.
    Conflict {
        file: String,
    },
    Io(std::io::Error),
}

impl Error {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::UnknownModel(_) => "unknown_model",
            Self::Busy(_) => "busy",
            Self::Canceled => "canceled",
            Self::HashMismatch { .. } => "hash_mismatch",
            Self::Network(_) => "network",
            Self::BadResponse(_) => "bad_response",
            Self::Conflict { .. } => "conflict",
            Self::Io(_) => "io",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownModel(id) => write!(f, "no model named `{id}` in the catalog"),
            Self::Busy(id) => write!(f, "another download of `{id}` is running"),
            Self::Canceled => write!(f, "download canceled"),
            Self::HashMismatch {
                file,
                expected,
                actual,
            } => write!(
                f,
                "`{file}` failed the SHA-256 check (expected {expected}, got {actual}) and was deleted"
            ),
            Self::Network(m) | Self::BadResponse(m) => f.write_str(m),
            Self::Conflict { file } => write!(
                f,
                "`{file}` is already there and is not the pinned file. AgentSI did not touch it"
            ),
            Self::Io(e) => write!(f, "disk error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
