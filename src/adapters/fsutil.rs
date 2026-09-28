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
/// and its directory as needed and keeping the final newline. Bytes that are not UTF-8 are kept
/// as they are. `true` when the file changed.
pub fn ensure_line_once(path: &Path, line: &str) -> Result<bool> {
    let mut data = read_regular(path)?.unwrap_or_default();
    if data.split(|&b| b == b'\n').any(|l| l == line.as_bytes()) {
        return Ok(false);
    }
    if !data.is_empty() && !data.ends_with(b"\n") {
        data.push(b'\n');
    }
    data.extend_from_slice(line.as_bytes());
    data.push(b'\n');
    if let Some(dir) = path.parent() {
        fs_err::create_dir_all(dir)?;
    }
    write_regular(path, &data)?;
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

    #[test]
    fn ensure_line_once_keeps_bytes_that_are_not_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("exclude");
        std::fs::write(&p, b"caf\xe9.tmp\n").unwrap();
        assert!(ensure_line_once(&p, "/a").unwrap());
        assert_eq!(std::fs::read(&p).unwrap(), b"caf\xe9.tmp\n/a\n");
    }
}
