//! `History` over `rev-list`, `merge-base`, `diff` and `log`.

use std::collections::HashSet;

use super::runner::{GitCli, lines_input};
use crate::application::ports::{History, RemoteScope, RevRange};
use crate::domain::error::{Error, Result};

impl History for GitCli {
    fn merge_base(&self, a: &str, b: &str) -> Result<Option<String>> {
        self.try_text(&["merge-base", a, b])
    }

    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool> {
        self.succeeds(&["merge-base", "--is-ancestor", ancestor, descendant])
    }

    fn list_range(&self, range: &RevRange) -> Result<Vec<String>> {
        let mut args: Vec<String> = ["rev-list", "--topo-order", "--reverse"]
            .map(String::from)
            .to_vec();
        args.push(range.tip.clone());
        args.extend(range.exclude_commits.iter().map(|c| format!("^{c}")));
        match &range.exclude_remotes {
            None => {}
            Some(RemoteScope::All) => args.extend(["--not".into(), "--remotes".into()]),
            Some(RemoteScope::Named(n)) => args.extend(["--not".into(), format!("--remotes={n}")]),
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.text(&refs)?;
        Ok(out
            .lines()
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect())
    }

    fn count_reachable(&self, rev: &str) -> Result<usize> {
        let out = self.text(&["rev-list", "--count", rev])?;
        parse_count(&out)
    }

    fn unpushed_among(&self, oids: &[String], upstream: &str) -> Result<HashSet<String>> {
        if oids.is_empty() {
            return Ok(HashSet::new());
        }
        let not = format!("^{upstream}");
        let out = self.run_stdin(
            &["rev-list", "--no-walk=unsorted", "--stdin", &not],
            lines_input(oids).as_bytes(),
        )?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .map(String::from)
            .collect())
    }

    fn all_reachable_from(&self, commits: &[String], tip: &str) -> Result<bool> {
        if commits.is_empty() {
            return Ok(true);
        }
        let mut input = lines_input(commits);
        input.push_str(&format!("^{tip}\n"));
        Ok(self
            .run_stdin(&["rev-list", "--stdin"], input.as_bytes())?
            .is_empty())
    }

    fn same_tree(&self, a: &str, b: &str) -> Result<bool> {
        self.succeeds(&["diff", "--quiet", a, b])
    }

    fn changed_paths(&self, a: &str, b: &str) -> Result<Vec<String>> {
        let out = self.run(&["diff-tree", "-r", "--name-only", "-z", a, b])?;
        Ok(out
            .split(|&c| c == 0)
            .filter(|p| !p.is_empty())
            .map(|p| String::from_utf8_lossy(p).to_string())
            .collect())
    }

    fn change_stats(&self, oids: &[String]) -> Result<Vec<(u64, u64, u64)>> {
        if oids.is_empty() {
            return Ok(Vec::new());
        }
        let out = self.run_stdin(
            &[
                "log",
                "--no-walk=unsorted",
                "--stdin",
                "--numstat",
                "--format=@@%H",
                "--diff-merges=first-parent",
            ],
            lines_input(oids).as_bytes(),
        )?;
        let text = String::from_utf8_lossy(&out);
        let mut stats = Vec::with_capacity(oids.len());
        let mut cur: Option<(u64, u64, u64)> = None;
        for l in text.lines() {
            if l.starts_with("@@") {
                stats.extend(cur.take());
                cur = Some((0, 0, 0));
            } else if let (Some(c), false) = (cur.as_mut(), l.is_empty()) {
                let mut it = l.split('\t');
                let (x, y) = (it.next().unwrap_or("-"), it.next().unwrap_or("-"));
                c.0 += x.parse::<u64>().unwrap_or(0);
                c.1 += y.parse::<u64>().unwrap_or(0);
                c.2 += 1;
            }
        }
        stats.extend(cur.take());
        if stats.len() != oids.len() {
            return Err(Error::Git(format!(
                "numstat returned {} entries for {} commits",
                stats.len(),
                oids.len()
            )));
        }
        Ok(stats)
    }
}

/// The number `git rev-list --count` printed; anything else is a git failure, not zero commits.
fn parse_count(out: &str) -> Result<usize> {
    out.trim().parse().map_err(|_| {
        Error::Git(format!(
            "`git rev-list --count` printed {out:?}, not a number"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_that_is_not_a_number_is_an_error() {
        assert_eq!(parse_count("42\n").unwrap(), 42);
        assert!(matches!(parse_count(""), Err(Error::Git(_))));
        assert!(matches!(parse_count("fatal"), Err(Error::Git(_))));
    }
}
