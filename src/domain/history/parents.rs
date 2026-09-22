//! Parents of a rewritten commit once dropped commits are skipped.

use std::collections::HashMap;

use super::plan::Parent;

/// Resolves `old_parents` for the new history: a parent that was rewritten becomes `In(index)`,
/// one that was dropped is replaced by its own (resolved) parents, anything else is kept as-is.
/// Duplicates that appear that way are removed, keeping the first.
pub fn resolve_parents(
    old_parents: &[String],
    rewritten: &HashMap<&str, usize>,
    dropped: &HashMap<&str, &[String]>,
) -> Vec<Parent> {
    let mut out: Vec<Parent> = Vec::new();
    let mut stack: Vec<&str> = old_parents.iter().rev().map(String::as_str).collect();
    while let Some(p) = stack.pop() {
        let resolved = if let Some(i) = rewritten.get(p) {
            Parent::In(*i)
        } else if let Some(grand) = dropped.get(p) {
            stack.extend(grand.iter().rev().map(String::as_str));
            continue;
        } else {
            Parent::Base(p.to_string())
        };
        if !out.contains(&resolved) {
            out.push(resolved);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn dropped_parents_are_replaced_by_their_parents() {
        let rewritten: HashMap<&str, usize> = [("a", 0)].into();
        let d1 = s(&["d2"]);
        let d2 = s(&["a"]);
        let dropped: HashMap<&str, &[String]> =
            [("d1", d1.as_slice()), ("d2", d2.as_slice())].into();
        assert_eq!(
            resolve_parents(&s(&["d1"]), &rewritten, &dropped),
            vec![Parent::In(0)]
        );
    }

    #[test]
    fn base_and_rewritten_pass_through_and_duplicates_collapse() {
        let rewritten: HashMap<&str, usize> = [("a", 0)].into();
        let d = s(&["a"]);
        let dropped: HashMap<&str, &[String]> = [("d", d.as_slice())].into();
        assert_eq!(
            resolve_parents(&s(&["x", "a", "d"]), &rewritten, &dropped),
            vec![Parent::Base("x".into()), Parent::In(0)]
        );
    }

    #[test]
    fn a_dropped_root_leaves_no_parent() {
        let none: Vec<String> = Vec::new();
        let dropped: HashMap<&str, &[String]> = [("r", none.as_slice())].into();
        assert!(resolve_parents(&s(&["r"]), &HashMap::new(), &dropped).is_empty());
    }
}
