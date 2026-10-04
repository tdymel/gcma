//! `History` over `rev-list`, `merge-base`, `diff` and `log`.

use std::collections::{HashMap, HashSet};

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
        args.extend(exclusions(range));
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.text(&refs)?;
        Ok(out
            .lines()
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect())
    }

    fn commits_off_branches(&self, tips: &[String]) -> Result<HashMap<String, Vec<String>>> {
        if tips.is_empty() {
            return Ok(HashMap::new());
        }
        // The tips go through stdin: every backup's commits are among them, more than a command
        // line holds. The `--not` there applies to `--branches` only, not to what stdin lists.
        let out = self.run_stdin(
            &["rev-list", "--parents", "--stdin", "--not", "--branches"],
            lines_input(tips).as_bytes(),
        )?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .filter_map(|line| {
                let mut oids = line.split_whitespace().map(String::from);
                Some((oids.next()?, oids.collect()))
            })
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

/// The `rev-list` arguments that leave out what the range excludes.
fn exclusions(range: &RevRange) -> Vec<String> {
    let mut args: Vec<String> = range
        .exclude_commits
        .iter()
        .map(|c| format!("^{c}"))
        .collect();
    match &range.exclude_remotes {
        None => {}
        Some(RemoteScope::All) => args.extend(["--not".into(), "--remotes".into()]),
        Some(RemoteScope::Named(n)) => args.extend(["--not".into(), format!("--remotes={n}")]),
    }
    args
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

    #[test]
    fn any_number_of_tips_fits() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let o = std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "Me")
                .env("GIT_AUTHOR_EMAIL", "me@home.org")
                .env("GIT_COMMITTER_NAME", "Me")
                .env("GIT_COMMITTER_EMAIL", "me@home.org")
                .stdin(std::process::Stdio::null())
                .output()
                .unwrap();
            assert!(o.status.success(), "{args:?}");
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        };
        git(&["init", "-q", "-b", "main"]);
        let tree = git(&["mktree"]);
        let commit = git(&["commit-tree", &tree, "-m", "off every branch"]);
        let repo = GitCli::open(dir.path()).unwrap();
        // About 4 MB of oids: more than a command line holds.
        let tips = vec![commit.clone(); 100_000];
        let graph = repo.commits_off_branches(&tips).unwrap();
        assert_eq!(graph.len(), 1);
        assert!(graph[&commit].is_empty());
    }
}
