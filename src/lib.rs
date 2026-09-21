//! ghma — "git hide my ass": safely redistribute commit times, identities and messages on the
//! current branch.

pub mod error;
pub mod git;
pub mod messages;
pub mod config;
pub mod schedule;
pub mod conform;
pub mod plan;
pub mod apply;
pub mod llm;
