use std::fmt;

#[derive(Debug)]
pub enum Error {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    /// A database file with a migration newer than this program knows.
    TooNew {
        found: u32,
        known: u32,
    },
    /// The writer thread is gone.
    Closed,
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Sqlite(e) => write!(f, "sqlite: {e}"),
            Error::Io(e) => write!(f, "{e}"),
            Error::TooNew { found, known } => {
                write!(
                    f,
                    "the database file has migration {found}, and this program knows only {known}"
                )
            }
            Error::Closed => write!(f, "the store writer has stopped"),
            Error::Other(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Error {
        Error::Sqlite(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::Io(e)
    }
}
