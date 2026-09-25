//! The `.gitignore` text that keeps removed paths out of the project.

/// The file the patterns are added to (always at the repository root).
pub const GITIGNORE: &str = ".gitignore";

const HEADER: &str = "# added by gcma: paths removed from history";

/// `existing` with every pattern appended that is not already a line of it. Idempotent; an
/// existing file is only ever extended, never reordered.
pub fn with_patterns(existing: Option<&[u8]>, patterns: &[String]) -> Vec<u8> {
    let mut out = existing.unwrap_or_default().to_vec();
    let has = |out: &[u8], line: &str| {
        out.split(|&b| b == b'\n')
            .any(|l| l.strip_suffix(b"\r").unwrap_or(l) == line.as_bytes())
    };
    let missing: Vec<&String> = patterns.iter().filter(|p| !has(&out, p)).collect();
    if missing.is_empty() {
        return out;
    }
    if !out.is_empty() && out.last() != Some(&b'\n') {
        out.push(b'\n');
    }
    if !has(&out, HEADER) {
        if !out.is_empty() {
            out.push(b'\n');
        }
        out.extend_from_slice(HEADER.as_bytes());
        out.push(b'\n');
    }
    for p in missing {
        out.extend_from_slice(p.as_bytes());
        out.push(b'\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn creates_the_file() {
        assert_eq!(
            with_patterns(None, &p(&["a/", "*.env"])),
            b"# added by gcma: paths removed from history\na/\n*.env\n"
        );
    }

    #[test]
    fn extends_without_touching_existing_lines() {
        let out = with_patterns(Some(b"target\nsecrets/"), &p(&["secrets/", "*.env"]));
        assert_eq!(
            out,
            b"target\nsecrets/\n\n# added by gcma: paths removed from history\n*.env\n"
        );
    }

    #[test]
    fn is_idempotent() {
        let once = with_patterns(Some(b"x\n"), &p(&["a", "b"]));
        assert_eq!(with_patterns(Some(&once), &p(&["a", "b"])), once);
        // A later pattern reuses the header.
        let more = with_patterns(Some(&once), &p(&["a", "b", "c"]));
        assert_eq!(more.iter().filter(|&&b| b == b'#').count(), 1);
        assert!(more.ends_with(b"c\n"));
    }

    #[test]
    fn no_patterns_is_identity() {
        assert_eq!(with_patterns(Some(b"x"), &[]), b"x");
        assert_eq!(with_patterns(None, &[]), b"");
    }
}
