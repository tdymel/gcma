//! The shape of a message: lines, paragraphs and the trailer block at the end.

pub(super) fn lines_of(msg: &[u8]) -> (Vec<&[u8]>, bool) {
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

pub(super) fn is_blank(l: &[u8]) -> bool {
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
pub(super) fn final_paragraph(lines: &[&[u8]]) -> Option<(usize, usize)> {
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

/// Applies `f` to the final paragraph when it is a trailer block; an empty result removes the
/// block and the blank lines before it. Anything else is returned unchanged.
/// One trailer line.
pub(super) type Line = Vec<u8>;

pub(super) fn map_final_block(msg: &[u8], f: &dyn Fn(&[Line]) -> Vec<Line>) -> Vec<u8> {
    let (orig, trailing_nl) = lines_of(msg);
    let Some((start, end)) = final_paragraph(&orig) else {
        return msg.to_vec();
    };
    if !orig[start..end].iter().all(|l| trailer_key(l).is_some()) {
        return msg.to_vec();
    }
    let block: Vec<Vec<u8>> = orig[start..end].iter().map(|l| l.to_vec()).collect();
    let mapped = f(&block);
    if mapped == block {
        return msg.to_vec();
    }
    let mut next: Vec<&[u8]> = Vec::new();
    if mapped.is_empty() {
        // Drop the block and the blank separator(s) before it; keep anything after it.
        let mut sep = start;
        while sep > 0 && is_blank(orig[sep - 1]) {
            sep -= 1;
        }
        next.extend_from_slice(&orig[..sep]);
    } else {
        next.extend_from_slice(&orig[..start]);
        next.extend(mapped.iter().map(Vec::as_slice));
    }
    next.extend_from_slice(&orig[end..]);
    let mut out = next.join(&b'\n');
    if trailing_nl {
        out.push(b'\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_trailers() {
        let m = b"S\n\nBody\n\nSigned-off-by: a\nCo-authored-by: b\n";
        assert_eq!(trailers(m).len(), 2);
        assert!(trailers(b"S\n\nBody line only\n").is_empty());
    }
}
