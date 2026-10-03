//! The commits a gcma rewrite replaced: per backup, what its `old` reaches and its `new` does not,
//! leaving out what a local branch still holds (the branch was restored, or another branch is built
//! on the commit).

use std::collections::{HashMap, HashSet};

/// The commits off every local branch, each with its parents (`History::commits_off_branches`).
pub(super) type Graph = HashMap<String, Vec<String>>;

/// The commits of `graph` that `start` reaches: none when `start` is on a branch. A parent outside
/// `graph` is on a branch, and so is everything it reaches.
pub(super) fn reach<'g>(graph: &'g Graph, start: &str) -> HashSet<&'g str> {
    let mut seen = HashSet::new();
    let mut todo: Vec<&str> = graph
        .get_key_value(start)
        .map(|(k, _)| k.as_str())
        .into_iter()
        .collect();
    while let Some(c) = todo.pop() {
        if !seen.insert(c) {
            continue;
        }
        let parents = graph[c]
            .iter()
            .filter_map(|p| graph.get_key_value(p.as_str()));
        todo.extend(parents.map(|(k, _)| k.as_str()));
    }
    seen
}

/// The replaced commits of the backups `(old, new)`. Taken per backup, not as every `old` minus
/// every `new`: a rewrite keeps the commits that already follow the rules, and after a chain of
/// hook rewrites one backup's `old` is built on the previous one's `new`, which it replaced.
pub(super) fn replaced(graph: &Graph, backups: &[(String, String)]) -> HashSet<String> {
    let mut out = HashSet::new();
    for (old, new) in backups {
        let olds = reach(graph, old);
        if olds.is_empty() {
            continue; // `old` is on a branch
        }
        let kept = reach(graph, new);
        out.extend(olds.difference(&kept).map(|c| c.to_string()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `edges` as (commit, parents), all off the branches.
    fn graph(edges: &[(&str, &[&str])]) -> Graph {
        edges
            .iter()
            .map(|(c, ps)| (c.to_string(), ps.iter().map(|p| p.to_string()).collect()))
            .collect()
    }

    fn sorted(set: HashSet<String>) -> Vec<String> {
        let mut v: Vec<String> = set.into_iter().collect();
        v.sort();
        v
    }

    fn pair(old: &str, new: &str) -> (String, String) {
        (old.to_string(), new.to_string())
    }

    #[test]
    fn what_the_rewrite_kept_is_not_replaced() {
        // `base` is on a branch; `kept` follows the rules, so `bad` was rewritten on top of it.
        let g = graph(&[("kept", &["base"]), ("bad", &["kept"]), ("bad2", &["kept"])]);
        assert_eq!(sorted(replaced(&g, &[pair("bad", "bad2")])), ["bad"]);
        assert_eq!(reach(&g, "base").len(), 0);
    }

    #[test]
    fn a_chain_of_hook_rewrites_replaces_every_earlier_new() {
        // a -> a1 (backup 1), then b on a1, and both re-timed: a1 -> a2, b -> b2 (backup 2).
        let g = graph(&[
            ("a", &[]),
            ("a1", &[]),
            ("b", &["a1"]),
            ("a2", &[]),
            ("b2", &["a2"]),
        ]);
        let backups = [pair("a", "a1"), pair("b", "b2")];
        assert_eq!(sorted(replaced(&g, &backups)), ["a", "a1", "b"]);
    }

    #[test]
    fn a_backup_whose_old_is_on_a_branch_replaces_nothing() {
        // After `restore` the branch holds `old` again, so it is not in the graph.
        let g = graph(&[("new", &["base"])]);
        assert!(replaced(&g, &[pair("old", "new")]).is_empty());
    }
}
