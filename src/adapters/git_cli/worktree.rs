//! `WorkTree` over `rev-parse`, `for-each-ref` and `diff --cached`.

use super::runner::GitCli;
use crate::application::ports::WorkTree;
use crate::domain::error::Result;

impl WorkTree for GitCli {
    fn is_shallow(&self) -> Result<bool> {
        Ok(self.text(&["rev-parse", "--is-shallow-repository"])? == "true")
    }

    fn has_replace_refs(&self) -> Result<bool> {
        Ok(!self
            .text(&["for-each-ref", "--count=1", "refs/replace"])?
            .is_empty())
    }

    fn has_grafts(&self) -> Result<bool> {
        Ok(self.git_path("info/grafts")?.exists())
    }

    fn index_dirty(&self) -> Result<bool> {
        Ok(!self.succeeds(&["diff", "--cached", "--quiet"])?)
    }

    fn operation_in_progress(&self) -> Result<Option<&'static str>> {
        for (p, name) in [
            ("rebase-merge", "rebase"),
            ("rebase-apply", "rebase/am"),
            ("MERGE_HEAD", "merge"),
            ("CHERRY_PICK_HEAD", "cherry-pick"),
            ("REVERT_HEAD", "revert"),
        ] {
            if self.git_path(p)?.exists() {
                return Ok(Some(name));
            }
        }
        Ok(None)
    }

    fn read_file(&self, rel: &str) -> Result<Option<Vec<u8>>> {
        match std::fs::read(self.dir().join(rel)) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn write_file(&self, rel: &str, data: &[u8]) -> Result<()> {
        Ok(std::fs::write(self.dir().join(rel), data)?)
    }

    fn reset_index_to_head(&self) -> Result<()> {
        self.run(&["reset", "-q", "--mixed", "HEAD", "--"])
            .map(|_| ())
    }
}
