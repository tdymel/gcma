//! Message transforms. All transforms are pure and idempotent on the message bytes.

fn lines_of(msg: &[u8]) -> (Vec<&[u8]>, bool) {
    let trailing_nl = msg.last() == Some(&b'\n');
    let body = if trailing_nl {
        &msg[..msg.len() - 1]
    } else {
        msg
    };
    if msg.is_empty() {
        return (Vec::new(), false);
    }
    (body.split(|&b| b == b'\n').collect(), trailing_nl)
}

fn is_blank(l: &[u8]) -> bool {
    l.iter().all(|b| b.is_ascii_whitespace())
}

/// Key of a `Key: value` trailer line.
pub fn trailer_key(l: &[u8]) -> Option<&[u8]> {
    let colon = l.iter().position(|&b| b == b':')?;
    let key = &l[..colon];
    let first = *key.first()?;
    if !first.is_ascii_alphabetic() || !key.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'-')
    {
        return None;
    }
    match l.get(colon + 1) {
        Some(b' ') | Some(b'\t') => Some(key),
        _ => None,
    }
}

/// Final paragraph (ignoring trailing blank lines) as a line range, unless it is the subject.
fn final_paragraph(lines: &[&[u8]]) -> Option<(usize, usize)> {
    let end = lines.iter().rposition(|l| !is_blank(l))? + 1;
    let mut start = end;
    while start > 0 && !is_blank(lines[start - 1]) {
        start -= 1;
    }
    if start == 0 {
        return None; // the first paragraph is the subject, never a trailer block
    }
    Some((start, end))
}

/// Trailer lines (as bytes) of the final paragraph, when every line of it is a trailer.
pub fn trailers(msg: &[u8]) -> Vec<Vec<u8>> {
    let (lines, _) = lines_of(msg);
    match final_paragraph(&lines) {
        Some((s, e)) if lines[s..e].iter().all(|l| trailer_key(l).is_some()) => {
            lines[s..e].iter().map(|l| l.to_vec()).collect()
        }
        _ => Vec::new(),
    }
}

/// Removes trailers whose key is in `keys` (case-insensitive) from the trailing trailer block(s),
/// repeating until nothing changes (so the function is idempotent).
pub fn strip_trailers(msg: &[u8], keys: &[String]) -> Vec<u8> {
    if keys.is_empty() {
        return msg.to_vec();
    }
    let (orig, trailing_nl) = lines_of(msg);
    let mut lines: Vec<Vec<u8>> = orig.iter().map(|l| l.to_vec()).collect();
    loop {
        let refs: Vec<&[u8]> = lines.iter().map(|l| l.as_slice()).collect();
        let Some((start, end)) = final_paragraph(&refs) else {
            break;
        };
        if !lines[start..end].iter().all(|l| trailer_key(l).is_some()) {
            break;
        }
        let kept: Vec<Vec<u8>> = lines[start..end]
            .iter()
            .filter(|l| {
                let k = trailer_key(l).unwrap();
                !keys.iter().any(|x| x.as_bytes().eq_ignore_ascii_case(k))
            })
            .cloned()
            .collect();
        if kept.len() == end - start {
            break;
        }
        let mut next: Vec<Vec<u8>> = Vec::new();
        if kept.is_empty() {
            // Drop the block and the blank separator(s) before it; keep anything after it.
            let mut sep = start;
            while sep > 0 && is_blank(&lines[sep - 1]) {
                sep -= 1;
            }
            next.extend_from_slice(&lines[..sep]);
            next.extend_from_slice(&lines[end..]);
        } else {
            next.extend_from_slice(&lines[..start]);
            next.extend(kept);
            next.extend_from_slice(&lines[end..]);
        }
        lines = next;
    }
    let mut out = lines.join(&b'\n');
    if trailing_nl {
        out.push(b'\n');
    }
    out
}

/// Appends each `Key: value` line of `add` that is not already present as a trailer (compared
/// byte-exact on the value, case-insensitive on the key). New lines join the final trailer block,
/// or start one after a blank line. Blank or empty messages are left alone.
pub fn add_trailers(msg: &[u8], add: &[String]) -> Vec<u8> {
    if add.is_empty() || msg.iter().all(u8::is_ascii_whitespace) {
        return msg.to_vec();
    }
    let present = trailers(msg);
    let missing: Vec<&String> = add
        .iter()
        .filter(|line| {
            let want = line.as_bytes();
            let key = trailer_key(want).unwrap_or(b"");
            let value = &want[(key.len() + 1).min(want.len())..];
            !present.iter().any(|p| {
                trailer_key(p).is_some_and(|k| k.eq_ignore_ascii_case(key))
                    && p[k_len(p) + 1..] == *value
            })
        })
        .collect();
    if missing.is_empty() {
        return msg.to_vec();
    }
    let mut out = msg.to_vec();
    if out.last() != Some(&b'\n') {
        out.push(b'\n');
    }
    if present.is_empty() {
        // A message that ends in a blank line keeps just one separator.
        while out.len() >= 2 && out[out.len() - 2] == b'\n' {
            out.pop();
        }
        out.push(b'\n');
    }
    for line in missing {
        out.extend_from_slice(line.as_bytes());
        out.push(b'\n');
    }
    out
}

fn k_len(trailer: &[u8]) -> usize {
    trailer_key(trailer).map_or(0, <[u8]>::len)
}

/// The message after the trailer rules: listed trailers dropped, wanted ones appended.
/// Idempotent as long as no key is both stripped and added (config validation guarantees it).
pub fn apply_trailer_rules(msg: &[u8], strip: &[String], add: &[String]) -> Vec<u8> {
    add_trailers(&strip_trailers(msg, strip), add)
}

/// First line of a message.
pub fn title(msg: &[u8]) -> String {
    let end = msg.iter().position(|&b| b == b'\n').unwrap_or(msg.len());
    String::from_utf8_lossy(&msg[..end]).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(keys: &[&str]) -> Vec<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    #[test]
    fn strips_listed_trailers() {
        let m = b"Subject\n\nBody\n\nSigned-off-by: a <a@b>\nCo-authored-by: c <c@d>\n";
        let out = strip_trailers(m, &s(&["signed-off-by"]));
        assert_eq!(out, b"Subject\n\nBody\n\nCo-authored-by: c <c@d>\n");
    }

    #[test]
    fn removes_empty_block_and_separator() {
        let m = b"Subject\n\nBody\n\nSigned-off-by: a\n";
        assert_eq!(
            strip_trailers(m, &s(&["Signed-off-by"])),
            b"Subject\n\nBody\n"
        );
    }

    #[test]
    fn repeats_until_fixed_point() {
        // After removing the last block, the previous paragraph is also a trailer block.
        let m = b"Subject\n\nSigned-off-by: a\n\nCo-authored-by: b\n";
        let once = strip_trailers(m, &s(&["Signed-off-by", "Co-authored-by"]));
        assert_eq!(once, b"Subject\n");
        assert_eq!(
            strip_trailers(&once, &s(&["Signed-off-by", "Co-authored-by"])),
            once
        );
    }

    #[test]
    fn idempotent_on_awkward_inputs() {
        let keys = s(&["Signed-off-by"]);
        for m in [
            &b"Signed-off-by: x"[..],
            b"Signed-off-by: x\n",
            b"Subject\n\nSigned-off-by: a\n\n\n",
            b"Subject\n\nSigned-off-by: a\nnot a trailer\n",
            b"",
            b"\n",
            b"S\n\nSigned-off-by: a\n\nBody after\n",
        ] {
            let once = strip_trailers(m, &keys);
            assert_eq!(
                strip_trailers(&once, &keys),
                once,
                "input {:?}",
                String::from_utf8_lossy(m)
            );
        }
    }

    #[test]
    fn subject_is_never_a_trailer_block() {
        assert_eq!(
            strip_trailers(b"Signed-off-by: x\n", &s(&["Signed-off-by"])),
            b"Signed-off-by: x\n"
        );
    }

    #[test]
    fn preserves_missing_trailing_newline() {
        let m = b"S\n\nBody\n\nSigned-off-by: a";
        assert_eq!(strip_trailers(m, &s(&["Signed-off-by"])), b"S\n\nBody");
    }

    #[test]
    fn no_keys_is_identity() {
        let m = b"S\r\n\nSigned-off-by: a\xff\n";
        assert_eq!(strip_trailers(m, &[]), m);
    }

    #[test]
    fn extracts_trailers() {
        let m = b"S\n\nBody\n\nSigned-off-by: a\nCo-authored-by: b\n";
        assert_eq!(trailers(m).len(), 2);
        assert!(trailers(b"S\n\nBody line only\n").is_empty());
    }

    #[test]
    fn adds_a_trailer_block_after_the_body() {
        let add = s(&["Assisted-By: Bot <b@x>"]);
        assert_eq!(
            add_trailers(b"Subject\n", &add),
            b"Subject\n\nAssisted-By: Bot <b@x>\n"
        );
        assert_eq!(
            add_trailers(b"Subject\n\nBody\n\n\n", &add),
            b"Subject\n\nBody\n\nAssisted-By: Bot <b@x>\n"
        );
        assert_eq!(
            add_trailers(b"Subject", &add),
            b"Subject\n\nAssisted-By: Bot <b@x>\n"
        );
    }

    #[test]
    fn joins_an_existing_trailer_block_and_is_idempotent() {
        let add = s(&["Assisted-By: Bot <b@x>"]);
        let once = add_trailers(b"S\n\nBody\n\nSigned-off-by: a\n", &add);
        assert_eq!(
            once,
            b"S\n\nBody\n\nSigned-off-by: a\nAssisted-By: Bot <b@x>\n"
        );
        assert_eq!(add_trailers(&once, &add), once);
        // Same key, different value: both are kept.
        let other = add_trailers(&once, &s(&["assisted-by: Other"]));
        assert!(other.ends_with(b"assisted-by: Other\n"));
    }

    #[test]
    fn strip_then_add_round_trips() {
        let m = b"S\n\nCo-Authored-By: Claude <c@x>\n";
        let out = apply_trailer_rules(m, &s(&["Co-Authored-By"]), &s(&["Assisted-By: Claude"]));
        assert_eq!(out, b"S\n\nAssisted-By: Claude\n");
        assert_eq!(
            apply_trailer_rules(&out, &s(&["Co-Authored-By"]), &s(&["Assisted-By: Claude"])),
            out
        );
    }

    #[test]
    fn empty_messages_are_left_alone() {
        assert_eq!(add_trailers(b"", &s(&["A: b"])), b"");
        assert_eq!(add_trailers(b"\n", &s(&["A: b"])), b"\n");
    }

    #[test]
    fn title_first_line() {
        assert_eq!(title(b"One\nTwo"), "One");
        assert_eq!(title(b""), "");
    }
}
