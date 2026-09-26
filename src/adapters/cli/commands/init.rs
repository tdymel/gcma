//! `gcma init`: write a starter config and keep it out of commits.

use crate::adapters::cli_support::session::Session;
use crate::adapters::config_file::{CONFIG_FILE, starter_config};
use crate::adapters::fsutil;
use crate::adapters::git_cli::GitCli;
use crate::domain::error::{Error, Result};

pub fn run(s: &Session, force: bool) -> Result<()> {
    let repo = GitCli::open(&s.start)?;
    let path = repo.dir().join(CONFIG_FILE);
    if path.exists() && !force {
        return Err(Error::Precondition(format!(
            "{} already exists (use --force)",
            path.display()
        )));
    }
    fsutil::write_regular(&path, starter_config().as_bytes())?;
    println!("wrote {}", path.display());
    // The config names identities and paths you want hidden: keep it out of commits.
    let exclude = repo.git_path("info/exclude")?;
    let line = format!("/{CONFIG_FILE}");
    if fsutil::ensure_line_once(&exclude, &line)? {
        println!("added {line} to .git/info/exclude so it is not committed by accident");
    }
    Ok(())
}
