//! One module per command. `cli` only parses and dispatches; each command opens what it needs
//! from the `Session` and hands the printing to `cli_support::report`.

pub mod apply;
pub mod hook;
pub mod init;
pub mod llm;
pub mod plan;
pub mod restore;
