//! Matching of tree paths against gitignore-style patterns.

use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::domain::error::{Error, Result};

/// Patterns in gitignore syntax, evaluated as if they were the root `.gitignore`.
#[derive(Debug, Clone)]
pub struct PathFilter {
    matcher: Gitignore,
    patterns: Vec<String>,
}

impl PathFilter {
    pub fn new(patterns: &[String]) -> Result<PathFilter> {
        let mut b = GitignoreBuilder::new("");
        for p in patterns {
            b.add_line(None, p)
                .map_err(|e| Error::Usage(format!("paths.exclude: invalid pattern {p:?}: {e}")))?;
        }
        let matcher = b
            .build()
            .map_err(|e| Error::Usage(format!("paths.exclude: {e}")))?;
        Ok(PathFilter {
            matcher,
            patterns: patterns.to_vec(),
        })
    }

    /// True when the entry at `path` (relative to the repository root, `/`-separated) is excluded.
    /// Directories are checked on their own, so an excluded directory takes its whole subtree.
    pub fn excludes(&self, path: &str, is_dir: bool) -> bool {
        self.matcher.matched(path, is_dir).is_ignore()
    }

    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(p: &[&str]) -> PathFilter {
        PathFilter::new(&p.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn directory_patterns_match_directories_only() {
        let f = f(&["secrets/"]);
        assert!(f.excludes("secrets", true));
        assert!(!f.excludes("secrets", false));
        assert!(f.excludes("a/secrets", true));
    }

    #[test]
    fn anchored_and_glob_patterns() {
        let f = f(&["/build", "*.env", "docs/*.tmp"]);
        assert!(f.excludes("build", true) && f.excludes("build", false));
        assert!(!f.excludes("src/build", true));
        assert!(f.excludes("prod.env", false) && f.excludes("deep/dir/prod.env", false));
        assert!(f.excludes("docs/x.tmp", false) && !f.excludes("docs/a/x.tmp", false));
    }

    #[test]
    fn negation_re_includes() {
        let f = f(&["*.log", "!keep.log"]);
        assert!(f.excludes("a.log", false));
        assert!(!f.excludes("keep.log", false));
    }

    #[test]
    fn no_patterns_exclude_nothing() {
        assert!(!f(&[]).excludes("anything", false));
    }
}
