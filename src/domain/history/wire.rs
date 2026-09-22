//! How byte strings appear in a plan file: JSON has no bytes, and commit messages and names need
//! not be UTF-8.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Serde module for `Vec<u8>` fields stored as base64 strings.
pub mod base64_bytes {
    use super::*;

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&B64.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        B64.decode(text).map_err(serde::de::Error::custom)
    }
}

/// A name or email: a plain JSON string when it is UTF-8, `{"base64": "..."}` otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Text(Vec<u8>);

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum Repr {
    Plain(String),
    Binary { base64: String },
}

impl Text {
    pub fn from_bytes(bytes: &[u8]) -> Text {
        Text(bytes.to_vec())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The text, when it is valid UTF-8.
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Text {
        Text(s.as_bytes().to_vec())
    }
}

impl From<String> for Text {
    fn from(s: String) -> Text {
        Text(s.into_bytes())
    }
}

/// Lossy: for messages and listings, never for writing objects.
impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&String::from_utf8_lossy(&self.0))
    }
}

impl Serialize for Text {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.as_str() {
            Some(plain) => Repr::Plain(plain.to_string()),
            None => Repr::Binary {
                base64: B64.encode(&self.0),
            },
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Text, D::Error> {
        match Repr::deserialize(d)? {
            Repr::Plain(s) => Ok(Text::from(s)),
            Repr::Binary { base64 } => B64
                .decode(base64)
                .map(Text)
                .map_err(serde::de::Error::custom),
        }
    }
}
