//! A random history for the randomized end-to-end check: branches, merges, octopus merges and odd
//! timestamps, with trailers in most messages.

use super::{Rand, Repo};

/// Two thirds of the commits carry sign-off and co-author trailers.
fn body(n: usize, subject: &str) -> String {
    if n % 3 == 1 {
        subject.to_string()
    } else {
        format!(
            "{subject}\n\nBody of {n}.\n\nSigned-off-by: Dev <dev@x.org>\nCo-authored-by: Pair <pair@x.org>"
        )
    }
}

impl Repo {
    /// Builds a random history of about `ops` steps from `rng`: commits by the old identity and by
    /// Jane Doe at odd, non-monotone times, new branches, and merges of one or two other branches
    /// (octopus merges). Ends checked out on `main`.
    pub fn random_dag(&self, rng: &mut Rand, ops: usize) {
        self.commit_as("root.txt", "root", 1_500_000_000, "Old Me", "me@home.org");
        let mut branches: Vec<String> = vec!["main".into()];
        let mut n = 0;
        for _ in 0..ops {
            n += 1;
            // Odd, non-monotone timestamps, sometimes far in the future.
            let t = 1_400_000_000 + (rng.below(400_000_000) as i64);
            let who = if rng.below(3) == 0 {
                ("Jane Doe", "jane@work.com")
            } else {
                ("Old Me", "me@home.org")
            };
            match rng.below(10) {
                0 | 1 => {
                    // New branch from a random existing commit.
                    let commits = self.git(&["rev-list", "--all"]);
                    let all: Vec<&str> = commits.lines().collect();
                    let from = all[rng.below(all.len() as u64) as usize];
                    let name = format!("b{n}");
                    self.git(&["checkout", "-q", "-b", &name, from]);
                    branches.push(name);
                    self.commit_as(
                        &format!("f{n}.txt"),
                        &body(n, &format!("branch commit {n}")),
                        t,
                        who.0,
                        who.1,
                    );
                }
                2 | 3 => {
                    // Switch branch.
                    let b = branches[rng.below(branches.len() as u64) as usize].clone();
                    self.git(&["checkout", "-q", &b]);
                    self.commit_as(
                        &format!("f{n}.txt"),
                        &body(n, &format!("commit {n}")),
                        t,
                        who.0,
                        who.1,
                    );
                }
                4 | 5 => {
                    // Merge one or two other branches into the current one (octopus when two).
                    let cur = self.git(&["rev-parse", "--abbrev-ref", "HEAD"]);
                    let mut others: Vec<String> =
                        branches.iter().filter(|b| **b != cur).cloned().collect();
                    if others.is_empty() {
                        self.commit_as(
                            &format!("f{n}.txt"),
                            &body(n, &format!("commit {n}")),
                            t,
                            who.0,
                            who.1,
                        );
                        continue;
                    }
                    let k = if others.len() > 1 && rng.below(3) == 0 {
                        2
                    } else {
                        1
                    };
                    let mut picked = Vec::new();
                    for _ in 0..k {
                        picked.push(others.remove(rng.below(others.len() as u64) as usize));
                    }
                    let date = format!("{t} +0000");
                    let mut args: Vec<&str> = vec!["merge", "-q", "--no-ff", "-m", "merge"];
                    args.extend(picked.iter().map(|s| s.as_str()));
                    let o = self
                        .cmd("git")
                        .args(&args)
                        .env("GIT_AUTHOR_DATE", &date)
                        .env("GIT_COMMITTER_DATE", &date)
                        .output()
                        .unwrap();
                    if !o.status.success() {
                        // Conflict or nothing to merge: clean up and carry on.
                        let _ = self.git_out(&["merge", "--abort"]);
                    }
                }
                _ => {
                    self.commit_as(
                        &format!("f{n}.txt"),
                        &body(n, &format!("commit {n}")),
                        t,
                        who.0,
                        who.1,
                    );
                }
            }
        }
        // End on main.
        self.git(&["checkout", "-q", "main"]);
    }
}
