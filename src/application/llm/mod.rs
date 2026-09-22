//! LLM-friendly message editing: which commits to show and strict validation of the replies.
//! The JSONL format itself lives in `adapters::llm_jsonl`.

mod export;
mod import;

pub use export::{Export, ExportRow, export};
pub use import::{ImportReport, Reply, ReplyLine, import};
