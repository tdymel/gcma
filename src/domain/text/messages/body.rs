//! The parts of a message that are not trailers: subject and body.

use super::block::{final_paragraph, is_blank, lines_of, trailer_key};

/// Keeps the subject paragraph and the final trailer block; the body in between is dropped.
pub fn title_only(msg: &[u8]) -> Vec<u8> {
    let (lines, trailing_nl) = lines_of(msg);
    let Some(subject_end) = lines.iter().position(|l| is_blank(l)) else {
        return msg.to_vec(); // nothing but a subject
    };
    if subject_end == 0 {
        return msg.to_vec(); // starts with a blank line: no subject to keep
    }
    let block = final_paragraph(&lines)
        .filter(|(s, e)| lines[*s..*e].iter().all(|l| trailer_key(l).is_some()));
    let mut kept: Vec<&[u8]> = lines[..subject_end].to_vec();
    if let Some((s, e)) = block {
        kept.push(b"");
        kept.extend_from_slice(&lines[s..e]);
    }
    let mut out = kept.join(&b'\n');
    if trailing_nl {
        out.push(b'\n');
    }
    out
}

/// First line of a message.
pub fn title(msg: &[u8]) -> String {
    let end = msg.iter().position(|&b| b == b'\n').unwrap_or(msg.len());
    String::from_utf8_lossy(&msg[..end]).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_only_keeps_the_subject_and_the_trailer_block() {
        assert_eq!(
            title_only(b"S\n\nbody one\n\nbody two\n\nA: b\n"),
            b"S\n\nA: b\n"
        );
        assert_eq!(title_only(b"S\n\nbody\n"), b"S\n");
        assert_eq!(title_only(b"S\n\nbody"), b"S");
        assert_eq!(title_only(b"S\n"), b"S\n");
        assert_eq!(
            title_only(b"S\n\nA: b\n"),
            b"S\n\nA: b\n",
            "no body, nothing to drop"
        );
        assert_eq!(
            title_only(b"Wrapped\nsubject\n\nbody\n"),
            b"Wrapped\nsubject\n",
            "the subject paragraph stays whole"
        );
        assert_eq!(title_only(b""), b"");
        assert_eq!(
            title_only(b"\n\nbody\n"),
            b"\n\nbody\n",
            "no subject: untouched"
        );
        // A body paragraph that merely looks like trailers is the last paragraph, so it is kept
        // as the trailer block (that is what git would call it too).
        assert_eq!(title_only(b"S\n\nbody\n\nSee: x\n"), b"S\n\nSee: x\n");
        let once = title_only(b"S\n\nbody\n\nA: b\n");
        assert_eq!(title_only(&once), once, "idempotent");
    }

    #[test]
    fn title_first_line() {
        assert_eq!(title(b"One\nTwo"), "One");
        assert_eq!(title(b""), "");
    }
}
