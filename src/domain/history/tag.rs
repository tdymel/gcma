//! Annotated tag objects: telling a signed one from an unsigned one, and pointing a copy at
//! another commit without touching anything else (name, tagger, date, message).

use crate::domain::error::{Error, Result};

/// The lines that open a signature block at the end of a tag message (what git itself looks for).
const SIGNATURE_MARKERS: &[&[u8]] = &[
    b"-----BEGIN PGP SIGNATURE-----",
    b"-----BEGIN PGP MESSAGE-----",
    b"-----BEGIN SIGNED MESSAGE-----",
    b"-----BEGIN SSH SIGNATURE-----",
];

/// The message of a raw tag object: everything after the first blank line.
fn message(raw: &[u8]) -> &[u8] {
    raw.windows(2)
        .position(|w| w == b"\n\n")
        .map_or(&[], |at| &raw[at + 2..])
}

/// Whether the raw tag object carries a signature. Any signature-looking line counts, so a doubtful
/// tag is left alone rather than silently stripped of its signature.
pub fn is_signed(raw: &[u8]) -> bool {
    message(raw)
        .split(|&b| b == b'\n')
        .any(|line| SIGNATURE_MARKERS.contains(&line))
}

/// The raw tag object with its target replaced by `new_target`; every other byte stays, so the tag
/// keeps its name, tagger, date and message. Only tags that point directly at `old_target`, a
/// commit, qualify.
pub fn retarget(raw: &[u8], old_target: &str, new_target: &str) -> Result<Vec<u8>> {
    let head = format!("object {old_target}\ntype commit\n");
    let rest = raw.strip_prefix(head.as_bytes()).ok_or_else(|| {
        Error::Internal(format!(
            "the tag object does not point directly at the commit {old_target}"
        ))
    })?;
    let mut out = format!("object {new_target}\ntype commit\n").into_bytes();
    out.extend_from_slice(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "1111111111111111111111111111111111111111";
    const NEW: &str = "2222222222222222222222222222222222222222";
    const PGP: &str = "-----BEGIN PGP SIGNATURE-----";

    fn tag_by(tagger: &str, message: &str) -> Vec<u8> {
        format!("object {OLD}\ntype commit\ntag v1\ntagger {tagger} <a@b.c> 1600000000 +0200\n\n{message}")
            .into_bytes()
    }

    fn tag(message: &str) -> Vec<u8> {
        tag_by("A B", message)
    }

    #[test]
    fn detects_every_kind_of_signature_block() {
        assert!(!is_signed(&tag("release 1\n")));
        assert!(!is_signed(&tag("")));
        for marker in SIGNATURE_MARKERS {
            let text = String::from_utf8_lossy(marker);
            assert!(
                is_signed(&tag(&format!("release\n{text}\nabc\n"))),
                "{text}"
            );
        }
    }

    #[test]
    fn a_marker_in_a_header_or_inside_a_line_is_no_signature() {
        assert!(!is_signed(&tag(&format!("see {PGP} above\n"))));
        assert!(!is_signed(&tag_by(PGP, "release\n")));
    }

    #[test]
    fn retarget_changes_only_the_object_line() {
        let raw = tag("release 1\n\nbody\n");
        let out = retarget(&raw, OLD, NEW).unwrap();
        let expected = String::from_utf8(raw).unwrap().replace(OLD, NEW);
        assert_eq!(String::from_utf8(out).unwrap(), expected);
    }

    #[test]
    fn retarget_refuses_other_targets_and_nested_tags() {
        assert!(retarget(&tag("m\n"), NEW, OLD).is_err());
        let nested = format!("object {OLD}\ntype tag\ntag v1\n\nm\n").into_bytes();
        assert!(retarget(&nested, OLD, NEW).is_err());
    }
}
