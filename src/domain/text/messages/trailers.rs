//! Trailer edits: strip by key, rewrite lines with regexes, append wanted ones.

use regex::bytes::Regex;

use super::block::{is_blank, map_final_block, trailer_key, trailers};

/// Removes trailers whose key is in `keys` (case-insensitive) from the trailing trailer block(s),
/// repeating until nothing changes (so the function is idempotent).
pub fn strip_trailers(msg: &[u8], keys: &[String]) -> Vec<u8> {
    if keys.is_empty() {
        return msg.to_vec();
    }
    let keep = |block: &[Vec<u8>]| -> Vec<Vec<u8>> {
        block
            .iter()
            .filter(|l| {
                let k = trailer_key(l).unwrap_or(b"");
                !keys.iter().any(|x| x.as_bytes().eq_ignore_ascii_case(k))
            })
            .cloned()
            .collect()
    };
    let mut cur = msg.to_vec();
    loop {
        let next = map_final_block(&cur, &keep);
        if next == cur {
            return cur;
        }
        cur = next;
    }
}

/// A sed-like rule for the lines of the trailer block: every match of `pattern` in a line is
/// replaced by `replacement` (`$1` refers to groups); a line that ends up empty is dropped.
#[derive(Debug, Clone)]
pub struct TrailerRewrite {
    pub pattern: Regex,
    pub replacement: String,
}

/// Rewrites the lines of the final trailer block with `rules`, in order. Lines that become
/// identical collapse into the first one.
pub fn rewrite_trailers(msg: &[u8], rules: &[TrailerRewrite]) -> Vec<u8> {
    if rules.is_empty() {
        return msg.to_vec();
    }
    let rewrite = |block: &[Vec<u8>]| -> Vec<Vec<u8>> {
        let mut out: Vec<Vec<u8>> = Vec::new();
        for line in block {
            let mut line = line.clone();
            for r in rules {
                line = r
                    .pattern
                    .replace_all(&line, r.replacement.as_bytes())
                    .into_owned();
            }
            if !line.is_empty() && !out.contains(&line) {
                out.push(line);
            }
        }
        out
    };
    map_final_block(msg, &rewrite)
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
    // Trailing blank lines would turn the new lines into a paragraph of their own and hide the
    // existing block, so they go; a message without a trailer block gets one separator back.
    while let Some(start) = last_line_start(&out) {
        if start == 0 || !is_blank(&out[start..out.len() - 1]) {
            break;
        }
        out.truncate(start);
    }
    if present.is_empty() {
        out.push(b'\n');
    }
    for line in missing {
        out.extend_from_slice(line.as_bytes());
        out.push(b'\n');
    }
    out
}

/// Offset of the last line of `buf`, which ends in a newline.
fn last_line_start(buf: &[u8]) -> Option<usize> {
    let body = buf.strip_suffix(b"\n")?;
    Some(body.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1))
}

fn k_len(trailer: &[u8]) -> usize {
    trailer_key(trailer).map_or(0, <[u8]>::len)
}

#[cfg(test)]
mod tests;
