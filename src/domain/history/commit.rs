//! The commit object model: byte-exact identities, headers and the raw commit buffer.

use crate::domain::error::{Error, Result};

/// A git identity line, byte-exact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawIdent {
    pub name: Vec<u8>,
    pub email: Vec<u8>,
    pub time: i64,
    /// UTC offset in minutes.
    pub tz: i32,
}

pub fn format_tz(tz: i32) -> String {
    let sign = if tz < 0 { '-' } else { '+' };
    let a = tz.abs();
    format!("{sign}{:02}{:02}", a / 60, a % 60)
}

fn parse_tz(s: &[u8]) -> Option<i32> {
    if s.len() != 5 {
        return None;
    }
    let sign = match s[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let d = &s[1..];
    if !d.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let num = |a: u8, b: u8| i32::from(a - b'0') * 10 + i32::from(b - b'0');
    Some(sign * (num(d[0], d[1]) * 60 + num(d[2], d[3])))
}

impl RawIdent {
    pub fn parse(v: &[u8]) -> Option<RawIdent> {
        let sp1 = v.iter().rposition(|&b| b == b' ')?;
        let tz = parse_tz(&v[sp1 + 1..])?;
        let rest = &v[..sp1];
        let sp2 = rest.iter().rposition(|&b| b == b' ')?;
        let time: i64 = std::str::from_utf8(&rest[sp2 + 1..]).ok()?.parse().ok()?;
        let who = &rest[..sp2];
        let gt = who.iter().rposition(|&b| b == b'>')?;
        let lt = who[..gt].iter().rposition(|&b| b == b'<')?;
        let mut name = who[..lt].to_vec();
        if name.last() == Some(&b' ') {
            name.pop();
        }
        Some(RawIdent {
            name,
            email: who[lt + 1..gt].to_vec(),
            time,
            tz,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.name);
        out.extend_from_slice(b" <");
        out.extend_from_slice(&self.email);
        out.extend_from_slice(b"> ");
        out.extend_from_slice(self.time.to_string().as_bytes());
        out.push(b' ');
        out.extend_from_slice(format_tz(self.tz).as_bytes());
        out
    }
}

/// An extra (non tree/parent/author/committer) header, kept verbatim including continuation lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub key: String,
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Commit {
    pub oid: String,
    pub tree: String,
    pub parents: Vec<String>,
    pub author: RawIdent,
    pub committer: RawIdent,
    pub extra: Vec<Header>,
    pub message: Vec<u8>,
}

impl Header {
    /// A signature over the commit.
    pub fn is_signature(&self) -> bool {
        self.key == "gpgsig" || self.key == "gpgsig-sha256"
    }

    /// Headers that cannot survive a rewrite: signatures and embedded signed tags.
    pub fn is_invalidated_by_rewrite(&self) -> bool {
        self.is_signature() || self.key == "mergetag"
    }
}

/// The first 8 characters of an object id, for messages and ids.
pub fn short(oid: &str) -> &str {
    &oid[..oid.len().min(8)]
}

impl Commit {
    pub fn has_signature(&self) -> bool {
        self.extra.iter().any(Header::is_signature)
    }

    pub fn parse(oid: &str, data: &[u8]) -> Result<Commit> {
        let bad = |what: &str| Error::Git(format!("commit {oid}: {what}"));
        let (head, message) = match memchr::memmem::find(data, b"\n\n") {
            Some(p) => (&data[..p], data[p + 2..].to_vec()),
            None => (data.strip_suffix(b"\n").unwrap_or(data), Vec::new()),
        };
        let mut raws: Vec<Vec<u8>> = Vec::new();
        for line in head.split(|&b| b == b'\n') {
            if line.first() == Some(&b' ') && !raws.is_empty() {
                let last = raws.last_mut().unwrap();
                last.push(b'\n');
                last.extend_from_slice(line);
            } else {
                raws.push(line.to_vec());
            }
        }
        let mut tree = None;
        let mut parents = Vec::new();
        let mut author = None;
        let mut committer = None;
        let mut extra = Vec::new();
        for raw in raws {
            let sp = raw.iter().position(|&b| b == b' ').unwrap_or(raw.len());
            let key = String::from_utf8_lossy(&raw[..sp]).to_string();
            let value = if sp < raw.len() {
                &raw[sp + 1..]
            } else {
                &raw[0..0]
            };
            match key.as_str() {
                "tree" if tree.is_none() => tree = Some(String::from_utf8_lossy(value).to_string()),
                "parent" => parents.push(String::from_utf8_lossy(value).to_string()),
                "author" if author.is_none() => {
                    author = Some(RawIdent::parse(value).ok_or_else(|| bad("bad author"))?)
                }
                "committer" if committer.is_none() => {
                    committer = Some(RawIdent::parse(value).ok_or_else(|| bad("bad committer"))?)
                }
                _ => extra.push(Header { key, raw }),
            }
        }
        Ok(Commit {
            oid: oid.to_string(),
            tree: tree.ok_or_else(|| bad("no tree"))?,
            parents,
            author: author.ok_or_else(|| bad("no author"))?,
            committer: committer.ok_or_else(|| bad("no committer"))?,
            extra,
            message,
        })
    }
}

/// A commit we want to create.
pub struct NewCommit<'a> {
    pub tree: &'a str,
    pub parents: &'a [String],
    pub author: &'a RawIdent,
    pub committer: &'a RawIdent,
    pub extra: Vec<&'a Header>,
    pub message: &'a [u8],
}

pub fn build_commit_buffer(c: &NewCommit) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(format!("tree {}\n", c.tree).as_bytes());
    for p in c.parents {
        out.extend_from_slice(format!("parent {p}\n").as_bytes());
    }
    out.extend_from_slice(b"author ");
    out.extend_from_slice(&c.author.to_bytes());
    out.push(b'\n');
    out.extend_from_slice(b"committer ");
    out.extend_from_slice(&c.committer.to_bytes());
    out.push(b'\n');
    for h in &c.extra {
        out.extend_from_slice(&h.raw);
        out.push(b'\n');
    }
    out.push(b'\n');
    out.extend_from_slice(c.message);
    out
}

pub fn is_zero_oid(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b == b'0')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ident_roundtrip() {
        let raw = b"Jane Q. Doe <jane@example.com> 1700000000 -0530";
        let id = RawIdent::parse(raw).unwrap();
        assert_eq!(id.name, b"Jane Q. Doe");
        assert_eq!(id.email, b"jane@example.com");
        assert_eq!(id.time, 1700000000);
        assert_eq!(id.tz, -330);
        assert_eq!(id.to_bytes(), raw);
    }

    #[test]
    fn ident_empty_name_and_odd_bytes() {
        let raw = b" <a@b> 5 +0000";
        let id = RawIdent::parse(raw).unwrap();
        assert_eq!(id.name, b"");
        let raw2 = b"N\xff <\xfe@b> 5 +0100";
        let id2 = RawIdent::parse(raw2).unwrap();
        assert_eq!(id2.to_bytes(), raw2);
    }

    #[test]
    fn commit_parse_keeps_multiline_headers() {
        let data = b"tree abc\nparent def\nauthor A <a@b> 1 +0000\ncommitter C <c@d> 2 +0200\nencoding latin1\ngpgsig -----BEGIN\n line2\n -----END\n\nmsg body\n\nlast";
        let c = Commit::parse("x", data).unwrap();
        assert_eq!(c.parents, vec!["def"]);
        assert_eq!(c.extra.len(), 2);
        assert_eq!(c.extra[1].key, "gpgsig");
        assert_eq!(c.extra[1].raw, b"gpgsig -----BEGIN\n line2\n -----END");
        assert_eq!(c.message, b"msg body\n\nlast");
        assert!(c.has_signature());
    }

    #[test]
    fn rebuild_is_byte_identical_without_signature() {
        let data = b"tree abc\nparent def\nauthor A <a@b> 1 +0000\ncommitter C <c@d> 2 +0200\nencoding latin1\n\nmsg\n";
        let c = Commit::parse("x", data).unwrap();
        let nc = NewCommit {
            tree: &c.tree,
            parents: &c.parents,
            author: &c.author,
            committer: &c.committer,
            extra: c.extra.iter().collect(),
            message: &c.message,
        };
        assert_eq!(build_commit_buffer(&nc), data);
    }

    #[test]
    fn zero_oid() {
        assert!(is_zero_oid("0000000000000000000000000000000000000000"));
        assert!(!is_zero_oid("0000a"));
    }
}
