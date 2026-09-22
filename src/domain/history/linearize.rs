//! Deterministic parents-first ordering of the commits to rewrite.

use std::collections::{BTreeSet, HashMap, HashSet};

use super::commit::Commit;

/// Deterministic parents-first order of the suffix, independent of dates:
/// ready commits are taken by (first-parent depth, oid).
pub fn linearize(suffix: &[String], commits: &HashMap<String, Commit>) -> Vec<String> {
    let set: HashSet<&String> = suffix.iter().collect();
    // First-parent depth within the suffix, computed independently of the input order.
    let mut depth: HashMap<&String, usize> = HashMap::new();
    for oid in suffix {
        let mut chain: Vec<&String> = Vec::new();
        let mut cur = oid;
        let mut base = 0;
        loop {
            if let Some(d) = depth.get(cur) {
                base = *d + 1;
                break;
            }
            chain.push(cur);
            match commits[cur].parents.first() {
                Some(p) if set.contains(p) => cur = p,
                _ => break,
            }
        }
        for (k, c) in chain.iter().rev().enumerate() {
            depth.insert(c, base + k);
        }
    }
    let mut indeg: HashMap<&String, usize> = HashMap::new();
    let mut children: HashMap<&String, Vec<&String>> = HashMap::new();
    for oid in suffix {
        let ps: BTreeSet<&String> = commits[oid]
            .parents
            .iter()
            .filter(|p| set.contains(p))
            .collect();
        indeg.insert(oid, ps.len());
        for p in ps {
            children.entry(p).or_default().push(oid);
        }
    }
    let mut ready: BTreeSet<(usize, &String)> = suffix
        .iter()
        .filter(|o| indeg[o] == 0)
        .map(|o| (depth[o], o))
        .collect();
    let mut out = Vec::with_capacity(suffix.len());
    while let Some(&(d, o)) = ready.iter().next() {
        ready.remove(&(d, o));
        out.push(o.clone());
        if let Some(ch) = children.get(o) {
            for c in ch {
                let n = indeg.get_mut(c).unwrap();
                *n -= 1;
                if *n == 0 {
                    ready.insert((depth[c], c));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::history::commit::RawIdent;

    fn c(oid: &str, parents: &[&str]) -> Commit {
        let id = RawIdent {
            name: b"n".to_vec(),
            email: b"e".to_vec(),
            time: 0,
            tz: 0,
        };
        Commit {
            oid: oid.into(),
            tree: "t".into(),
            parents: parents.iter().map(|s| s.to_string()).collect(),
            author: id.clone(),
            committer: id,
            extra: Vec::new(),
            message: Vec::new(),
        }
    }

    #[test]
    fn linearize_is_parents_first_and_deterministic() {
        // a <- b <- d (merge of b and c) ; c <- a
        let commits: HashMap<String, Commit> = [
            c("a", &[]),
            c("b", &["a"]),
            c("c", &["a"]),
            c("d", &["b", "c"]),
        ]
        .into_iter()
        .map(|x| (x.oid.clone(), x))
        .collect();
        let s: Vec<String> = ["d", "c", "b", "a"].iter().map(|x| x.to_string()).collect();
        let lin = linearize(&s, &commits);
        let pos = |o: &str| lin.iter().position(|x| x == o).unwrap();
        assert!(
            pos("a") < pos("b")
                && pos("a") < pos("c")
                && pos("b") < pos("d")
                && pos("c") < pos("d")
        );
        assert_eq!(lin, linearize(&s, &commits));
        let s2: Vec<String> = ["a", "b", "c", "d"].iter().map(|x| x.to_string()).collect();
        assert_eq!(lin, linearize(&s2, &commits), "independent of input order");
    }
}
