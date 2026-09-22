use thiserror::Error;

/// Exit codes: 0 ok/no-op, 1 internal, 2 usage/config, 3 precondition refused,
/// 4 tip moved, 5 pushed commits in the suffix, 6 nonconforming, 7 LLM reply invalid.
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

impl Error {
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Internal(_) | Error::Git(_) | Error::Io(_) => 1,
            Error::Usage(_) => 2,
            Error::Precondition(_) => 3,
            Error::TipMoved(_) => 4,
            Error::Pushed(_) => 5,
            Error::Nonconforming(_) => 6,
            Error::LlmInvalid(_) => 7,
        }
    }
}

impl From<base64::DecodeError> for Error {
    fn from(e: base64::DecodeError) -> Self {
        Error::Usage(format!("base64: {e}"))
    }
}
