//! File writes that never follow a symlink out of the place they are meant for.

use std::path::Path;

use crate::domain::error::{Error, Result};

fn refuse_symlink(path: &Path) -> Result<()> {
    match fs_err::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Err(Error::Precondition(format!(
            "{} is a symbolic link; refusing to read or write through it",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Reads the file; `None` when it does not exist. Refuses symbolic links.
pub fn read_regular(path: &Path) -> Result<Option<Vec<u8>>> {
    refuse_symlink(path)?;
    match fs_err::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Writes the file. Refuses symbolic links.
pub fn write_regular(path: &Path, data: &[u8]) -> Result<()> {
    refuse_symlink(path)?;
    Ok(fs_err::write(path, data)?)
}

/// Removes the file if it exists. Refuses symbolic links.
pub fn remove_regular(path: &Path) -> Result<()> {
    refuse_symlink(path)?;
    match fs_err::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Appends `line` to the text file at `path` unless a line equals it already, creating the file
/// and its directory as needed and keeping the final newline. `true` when the file changed.
pub fn ensure_line_once(path: &Path, line: &str) -> Result<bool> {
    let current = read_regular(path)?.unwrap_or_default();
    let mut text = String::from_utf8_lossy(&current).into_owned();
    if text.lines().any(|l| l == line) {
        return Ok(false);
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(line);
    text.push('\n');
    if let Some(dir) = path.parent() {
        fs_err::create_dir_all(dir)?;
    }
    write_regular(path, text.as_bytes())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_line_once_creates_appends_and_dedupes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("info/exclude");
        assert!(ensure_line_once(&p, "/a").unwrap());
        assert!(!ensure_line_once(&p, "/a").unwrap());
        std::fs::write(&p, "x\n/a").unwrap();
        assert!(ensure_line_once(&p, "/b").unwrap());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "x\n/a\n/b\n");
    }
}
