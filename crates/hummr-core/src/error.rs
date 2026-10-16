use std::path::PathBuf;

/// Errors produced by the audio engine. Messages are written for end users:
/// they say what went wrong and, where possible, what to do about it.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not access {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{} is not a usable WAV file: {reason}", path.display())]
    Wav { path: PathBuf, reason: String },

    #[error("MIDI error: {0}")]
    Midi(String),

    #[error("invalid {what} '{input}': {hint}")]
    Parse {
        what: &'static str,
        input: String,
        hint: String,
    },

    #[error("{0}")]
    Audio(String),

    #[error("{0}")]
    Project(String),
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn parse(what: &'static str, input: &str, hint: impl Into<String>) -> Self {
        Error::Parse {
            what,
            input: input.to_string(),
            hint: hint.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
