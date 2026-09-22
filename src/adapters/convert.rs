//! Conversions from the errors of file-format libraries into the domain error.

use crate::domain::error::Error;

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Usage(format!("json: {e}"))
    }
}

impl From<serde_yaml::Error> for Error {
    fn from(e: serde_yaml::Error) -> Self {
        Error::Usage(format!("config: {e}"))
    }
}
