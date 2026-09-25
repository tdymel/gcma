//! `ghma init`: write a starter config and keep it out of commits.

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
    let current = fsutil::read_regular(&exclude)?.unwrap_or_default();
    if !String::from_utf8_lossy(&current).lines().any(|l| l == line) {
        let mut text = String::from_utf8_lossy(&current).to_string();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&line);
        text.push('\n');
        if let Some(dir) = exclude.parent() {
            std::fs::create_dir_all(dir)?;
        }
        fsutil::write_regular(&exclude, text.as_bytes())?;
        println!("added {line} to .git/info/exclude so it is not committed by accident");
    }
    Ok(())
}
