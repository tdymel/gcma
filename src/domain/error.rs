use thiserror::Error;

/// What went wrong; the command line maps each kind to its exit code.
#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Internal(String),
    #[error("{0}")]
    Usage(String),
    #[error("{0}")]
    Precondition(String),
    #[error("{0}")]
    TipMoved(String),
    #[error("{0}")]
    Pushed(String),
    #[error("{0}")]
    Nonconforming(String),
    #[error("{0}")]
    LlmInvalid(String),
    #[error("git: {0}")]
    Git(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
