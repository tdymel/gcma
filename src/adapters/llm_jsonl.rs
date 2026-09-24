//! The JSONL format an LLM reads and writes: one object per commit.

use serde::{Deserialize, Serialize};

use crate::application::llm::{Export, Reply, ReplyLine};
use crate::domain::error::Result;

pub const PROMPT: &str = "\
You will receive JSONL, one commit per line: {\"i\": index, \"d\": date, \"s\": \"+adds-dels Nf\", \"m\": current message}.
(\"a\" appears only when the author differs from the common author named below.)
Write a better commit message for each commit you can improve: a concise imperative title (<= 72 chars, one line)
and an optional body explaining why. Judge from the current message and the stat; do not invent facts.
Reply with JSONL ONLY (no prose, no code fences), one object per commit you change:
{\"i\": <same index>, \"t\": \"<title>\", \"b\": \"<body, optional>\"}
Omit commits whose message is already good. Keep every Signed-off-by / Co-authored-by line of the original message
in your reply (they are protected: a reply that drops one is rejected).";

#[derive(Serialize)]
struct OutRow<'a> {
    i: usize,
    d: &'a str,
    /// `+adds-dels Nf`.
    s: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    a: Option<&'a str>,
    m: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InRow {
    i: usize,
    t: String,
    b: Option<String>,
}

/// The prelude (instructions, for stderr) and the JSONL rows (for stdout).
pub fn render_export(export: &Export) -> Result<(String, Vec<String>)> {
    let rows = export
        .rows
        .iter()
        .map(|r| {
            serde_json::to_string(&OutRow {
                i: r.index,
                d: &r.date,
                s: format!("+{}-{} {}f", r.added, r.removed, r.files),
                a: r.author.as_deref(),
                m: &r.message,
            })
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let prelude = format!(
        "{PROMPT}\n\nCommon author: {}\nRows {}..{} of {}.\n",
        export.common_author, export.offset, export.end, export.total
    );
    Ok((prelude, rows))
}

/// One result per non-empty line; prose, code fences and unknown keys are errors.
pub fn parse_reply(text: &str) -> Vec<ReplyLine> {
    text.lines()
        .enumerate()
        .map(|(n, line)| (n + 1, line.trim()))
        .filter(|(_, line)| !line.is_empty())
        .map(|(n, line)| {
            serde_json::from_str::<InRow>(line)
                .map(|r| Reply {
                    index: r.i,
                    title: r.t,
                    body: r.b,
                })
                .map_err(|e| {
                    format!("line {n}: not a valid reply row ({e}); prose or code fences are not allowed")
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::llm::ExportRow;

    #[test]
    fn rows_parse_with_and_without_a_body() {
        let rows =
            parse_reply("{\"i\":0,\"t\":\"A\",\"b\":\"why\"}\n\n  {\"i\":3,\"t\":\"B\"}  \n");
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            Ok(Reply {
                index: 0,
                title: "A".into(),
                body: Some("why".into())
            })
        );
        assert_eq!(rows[1].as_ref().unwrap().index, 3);
    }

    #[test]
    fn prose_fences_and_unknown_keys_are_errors_with_their_line_number() {
        for bad in [
            "Sure! Here you go:",
            "```json",
            "{\"i\":0,\"t\":\"x\",\"z\":1}",
            "{\"i\":\"0\",\"t\":\"x\"}",
        ] {
            let rows = parse_reply(&format!("{{\"i\":0,\"t\":\"ok\"}}\n{bad}"));
            assert!(rows[0].is_ok());
            let e = rows[1].as_ref().unwrap_err();
            assert!(e.starts_with("line 2:"), "{e}");
        }
    }

    #[test]
    fn json_escapes_decode_so_the_importer_can_see_control_characters() {
        let rows = parse_reply("{\"i\":0,\"t\":\"x\\u0000y\"}");
        assert_eq!(rows[0].as_ref().unwrap().title, "x\0y");
    }

    #[test]
    fn export_rows_are_compact_and_name_the_author_only_when_it_differs() {
        let export = Export {
            common_author: "A <a@x>".into(),
            offset: 0,
            end: 2,
            total: 2,
            rows: vec![
                ExportRow {
                    index: 0,
                    date: "2026-01-02".into(),
                    added: 3,
                    removed: 1,
                    files: 2,
                    author: None,
                    message: "wip \"quoted\"".into(),
                },
                ExportRow {
                    index: 1,
                    date: "2026-01-03".into(),
                    added: 0,
                    removed: 0,
                    files: 0,
                    author: Some("B <b@x>".into()),
                    message: "two\nlines".into(),
                },
            ],
        };
        let (prelude, rows) = render_export(&export).unwrap();
        assert!(prelude.contains("Common author: A <a@x>"));
        assert!(prelude.contains("Rows 0..2 of 2."));
        assert_eq!(
            rows[0],
            "{\"i\":0,\"d\":\"2026-01-02\",\"s\":\"+3-1 2f\",\"m\":\"wip \\\"quoted\\\"\"}"
        );
        assert!(rows[1].contains("\"a\":\"B <b@x>\""));
        assert!(rows[1].contains("two\\nlines"));
    }
}
