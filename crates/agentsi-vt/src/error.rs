use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    OutOfMemory,
    InvalidValue,
    /// The buffer was too small. `required` is the size that fits, or 0 when the library did not say.
    OutOfSpace {
        required: usize,
    },
    /// The library returned this code, which this crate does not know.
    Code(i32),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::OutOfMemory => write!(f, "libghostty-vt is out of memory"),
            Error::InvalidValue => write!(f, "libghostty-vt rejected a value"),
            Error::OutOfSpace { required } => {
                write!(f, "buffer too small, {required} bytes needed")
            }
            Error::Code(code) => write!(f, "libghostty-vt returned code {code}"),
        }
    }
}

impl std::error::Error for Error {}
