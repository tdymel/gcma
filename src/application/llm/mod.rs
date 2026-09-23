//! LLM-friendly message editing: which commits to show and strict validation of the replies.
//! The JSONL format itself lives in `adapters::llm_jsonl`.

mod export;
mod import;

#[cfg(test)]
pub use export::ExportRow;
pub use export::{Export, export};
pub use import::{Reply, ReplyLine, import};
