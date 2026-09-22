//! Path rules during planning: new trees, dropped commits and where `.gitignore` is added.

use std::collections::{HashMap, HashSet};

use crate::application::pathrules::TreeRewriter;
use crate::domain::error::Result;
use crate::domain::history::commit::Commit;
use crate::domain::history::parents::resolve_parents;
use crate::domain::history::plan::Parent;
use crate::domain::settings::{Config, OnlyExcluded};

/// What the path rules do to the commits to rewrite.
#[derive(Debug, Default)]
pub(super) struct PathOutcome {
    /// Commits that stay, parents first.
    pub kept: Vec<String>,
    /// Commits that only touched excluded paths (parents first).
    pub dropped: Vec<String>,
    /// New tree and `.gitignore` flag of every kept commit.
    pub trees: HashMap<String, (String, bool)>,
}

impl PathOutcome {
    /// Without path rules every commit stays as it is.
    pub(super) fn untouched(linear: &[String]) -> PathOutcome {
        PathOutcome {
            kept: linear.to_vec(),
            ..PathOutcome::default()
        }
    }
}

/// `linear` is the suffix, parents first. `commits` holds the suffix and its external parents.
/// `.gitignore` gets the patterns in the first kept commit at or after a commit that had excluded
/// paths, and in everything built on top of it, so the files stay out of git from then on.
///
/// When the tip itself only touched excluded paths and nothing kept on the way carries the
/// patterns, the tip stays (as a commit that only adds them): the project still has those files.
pub(super) fn apply_rules(
    rewriter: &TreeRewriter,
    cfg: &Config,
    linear: &[String],
    commits: &HashMap<String, Commit>,
    excluded: &HashSet<String>,
    tip: &str,
) -> Result<PathOutcome> {
    let (outcome, flagged) = pass(rewriter, cfg, linear, commits, excluded, None)?;
    let tip_lost_the_patterns = cfg.paths.gitignore
        && excluded.contains(tip)
        && outcome.dropped.iter().any(|o| o == tip)
        && !new_tip_carries_patterns(rewriter, &outcome, commits, tip, &flagged)?;
    if tip_lost_the_patterns {
        return Ok(pass(rewriter, cfg, linear, commits, excluded, Some(tip))?.0);
    }
    Ok(outcome)
}

/// Is the commit the branch ends up on (after dropping the tip) one with the patterns in it?
fn new_tip_carries_patterns(
    rewriter: &TreeRewriter,
    outcome: &PathOutcome,
    commits: &HashMap<String, Commit>,
    tip: &str,
    flagged: &HashSet<String>,
) -> Result<bool> {
    let index: HashMap<&str, usize> = outcome
        .kept
        .iter()
        .enumerate()
        .map(|(i, o)| (o.as_str(), i))
        .collect();
    let dropped: HashMap<&str, &[String]> = outcome
        .dropped
        .iter()
        .map(|o| (o.as_str(), commits[o].parents.as_slice()))
        .collect();
    Ok(
        match resolve_parents(&commits[tip].parents, &index, &dropped).first() {
            Some(Parent::In(i)) => {
                outcome.trees[&outcome.kept[*i]].1 && flagged.contains(&outcome.kept[*i])
            }
            // A commit that stays as it is carries them if its .gitignore already has them.
            Some(Parent::Base(b)) => {
                let tree = &commits[b].tree;
                rewriter.with_gitignore(tree)? == *tree
            }
            None => false,
        },
    )
}

fn pass(
    rewriter: &TreeRewriter,
    cfg: &Config,
    linear: &[String],
    commits: &HashMap<String, Commit>,
    excluded: &HashSet<String>,
    force_keep: Option<&str>,
) -> Result<(PathOutcome, HashSet<String>)> {
    let empty = rewriter.empty_tree()?;
    let mut out = PathOutcome::default();
    let mut flagged: HashSet<String> = HashSet::new();
    for oid in linear {
        let c = &commits[oid];
        let flag = excluded.contains(oid) || c.parents.iter().any(|p| flagged.contains(p));
        if flag {
            flagged.insert(oid.clone());
        }
        let filtered = rewriter.without_excluded(&c.tree)?;
        if cfg.paths.only_excluded_commits == OnlyExcluded::Drop
            && c.parents.len() <= 1
            && force_keep != Some(oid.as_str())
        {
            let (parent_tree, parent_filtered) = match c.parents.first() {
                Some(p) => {
                    let pt = &commits[p].tree;
                    (pt.clone(), rewriter.without_excluded(pt)?)
                }
                None => (empty.clone(), empty.clone()),
            };
            // Nothing outside the excluded paths changed, but something did: drop it. A commit
            // that was empty to begin with is kept.
            if filtered == parent_filtered && c.tree != parent_tree {
                out.dropped.push(oid.clone());
                continue;
            }
        }
        let mut tree = filtered;
        let add_ignore = flag && cfg.paths.gitignore;
        if add_ignore {
            tree = rewriter.with_gitignore(&tree)?;
        }
        out.trees.insert(oid.clone(), (tree, add_ignore));
        out.kept.push(oid.clone());
    }
    Ok((out, flagged))
}
