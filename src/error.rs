use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{0}")]
    Config(String),
    #[error("{operation}: {message}")]
    Runtime { operation: String, message: String },
    #[error("initialization failed: {cause}; cleanup failed: {cleanup}")]
    Cleanup {
        cause: Box<Error>,
        cleanup: Box<Error>,
    },
    #[error("{0}")]
    Multiple(String),
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub fn runtime(operation: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Runtime {
            operation: operation.into(),
            message: message.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
