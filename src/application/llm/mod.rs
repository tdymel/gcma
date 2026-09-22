//! LLM-friendly message editing: compact export and strict import.

mod export;
mod import;

pub use export::{PROMPT, export};
pub use import::{ImportReport, import};
