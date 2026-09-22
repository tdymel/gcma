//! `WorkTree` over `rev-parse`, `for-each-ref` and `diff --cached`.

use super::GitCli;
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
}
