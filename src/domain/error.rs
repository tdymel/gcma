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
    /// The schedule has no allowed time left for the commits to place. A precondition too (exit 3),
    /// told apart so a caller that must stay quiet (the post-commit hook) can skip it.
    #[error("{0}")]
    NoCapacity(String),
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

impl Error {
    /// A check of what a rewrite wrote failed, before any ref moved.
    pub fn verification(what: impl std::fmt::Display) -> Error {
        Error::Internal(format!("verification failed, no ref was changed: {what}"))
    }
}

pub type Result<T> = std::result::Result<T, Error>;
