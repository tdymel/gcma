//! Path rules: which paths are removed from history, and the `.gitignore` text that keeps them
//! out of the project afterwards. Patterns use gitignore syntax.

mod filter;
mod gitignore;

pub use filter::PathFilter;
pub use gitignore::{GITIGNORE, with_patterns};
