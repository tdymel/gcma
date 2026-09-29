//! Non-fatal warnings about the commits a plan would rewrite.

use std::collections::HashMap;

use crate::application::ports::Repository;
use crate::domain::error::Result;
use crate::domain::history::commit::Commit;
use crate::domain::settings::{Config, Signing};

/// Non-fatal consequences of rewriting these commits.
pub(super) fn rewrite_warnings(
    repo: &dyn Repository,
    cfg: &Config,
    linear: &[String],
    commits: &HashMap<String, Commit>,
) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    let labels = repo.labels_pointing_at(&linear.iter().cloned().collect())?;
    if !labels.is_empty() {
        warnings.push(format!(
            "tags/notes point at commits that will be rewritten and will keep pointing at the old ones: {}",
            labels.join(", ")
        ));
    }
    if cfg.signing == Signing::Resign {
        let lossy = linear
            .iter()
            .filter(|o| {
                commits[*o]
                    .extra
                    .iter()
                    .any(|h| !h.is_invalidated_by_rewrite())
            })
            .count();
        if lossy > 0 {
            warnings.push(format!("{lossy} commit(s) carry extra headers (e.g. encoding) that `signing: resign` cannot preserve"));
        }
        let binary = linear
            .iter()
            .filter(|o| std::str::from_utf8(&commits[*o].message).is_err())
            .count();
        if binary > 0 {
            warnings.push(format!(
                "{binary} commit(s) have messages that are not valid UTF-8; git recodes those when it signs, so `apply` \
                 refuses them with `signing: resign` unless an imported reply gives them a new message (or use `signing: strip`)"
            ));
        }
    }
    Ok(warnings)
}
