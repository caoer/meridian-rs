//! The strict reader the G12 law describes: it consumes each sections body by
//! the head's declared `bytes` and never scans for a boundary. Shared by the
//! `main` binary's G12 module and the `g12_stored_link` binary.

use serde_json::{Value, json};

/// One row of the head's `sections` array, reduced to what the boundary law
/// needs: the marker ordinal and the declared body length.
struct DeclaredRow {
    n: String,
    bytes: usize,
}

/// Split a TOON tabular row on commas that are OUTSIDE quotes.
fn row_cells(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    for c in line.chars() {
        match c {
            _ if escaped => {
                cur.push(c);
                escaped = false;
            }
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => cells.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    cells.push(cur);
    cells
}

/// Read the head's declarations: the `sections[N]{...}` array header, its
/// column names, and the N rows that follow it.
///
/// The head is a TOON document and contains no blank line, so the FIRST `\n\n`
/// in the face is unambiguously the head/body separator — nothing here scans
/// body territory for anything.
fn declared_rows(head: &str) -> Vec<DeclaredRow> {
    let mut lines = head.lines();
    let header = loop {
        let Some(line) = lines.next() else {
            panic!("the head declares no `sections` array:\n{head}");
        };
        if line == "sections: []" {
            return Vec::new();
        }
        if line.starts_with("sections[") {
            break line;
        }
    };

    let (count, cols) = {
        let open = header.find('[').expect("array header has `[`");
        let close = header.find(']').expect("array header has `]`");
        let count: usize = header[open + 1..close]
            .parse()
            .unwrap_or_else(|e| panic!("`{header}` declares a row count ({e})"));
        let brace_open = header.find('{').expect("array header has `{`");
        let brace_close = header.find('}').expect("array header has `}`");
        let cols: Vec<&str> = header[brace_open + 1..brace_close].split(',').collect();
        (count, cols)
    };
    let n_col = cols.iter().position(|c| *c == "n").expect("an `n` column");
    let bytes_col = cols
        .iter()
        .position(|c| *c == "bytes")
        .expect("a `bytes` column");

    (0..count)
        .map(|i| {
            let line = lines
                .next()
                .unwrap_or_else(|| panic!("`{header}` promised {count} rows, row {i} is missing"));
            let cells = row_cells(line.trim_start());
            assert_eq!(
                cells.len(),
                cols.len(),
                "row {i} has {} cells for {} columns: {line}",
                cells.len(),
                cols.len()
            );
            DeclaredRow {
                n: cells[n_col].clone(),
                bytes: cells[bytes_col]
                    .parse()
                    .unwrap_or_else(|e| panic!("row {i} declares a byte length ({e}): {line}")),
            }
        })
        .collect()
}

/// **The property.** Parse the face the way the law says a reader must — by
/// declared length, never by marker — and require the bytes served to equal the
/// bytes declared, for every body, with nothing left over.
///
/// Returns the bodies it consumed, so a caller can make claims about them.
pub(crate) fn bodies_by_declared_length(case: &str, text: &str) -> Vec<String> {
    let head_end = text
        .find("\n\n")
        .unwrap_or_else(|| panic!("[{case}] the face has no head/body separator:\n{text}"));
    let rows = declared_rows(&text[..head_end]);

    let mut cur = head_end;
    let mut bodies = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        assert!(
            text[cur..].starts_with("\n\n"),
            "[{case}] body {i} is not opened by a blank line at byte {cur}"
        );
        cur += 2;
        let nl = text[cur..]
            .find('\n')
            .unwrap_or_else(|| panic!("[{case}] body {i}'s marker line is unterminated"));
        assert_eq!(
            &text[cur..cur + nl],
            &format!("== {} ==", row.n),
            "[{case}] body {i}'s marker names its declared ordinal"
        );
        cur += nl + 1;

        let end = cur + row.bytes;
        let body = text.get(cur..end).unwrap_or_else(|| {
            panic!(
                "[{case}] body {i} (n={}) declares {} bytes but the face serves only {} after its \
                 marker — the head's `bytes` IS the boundary (W1/U15), so a body shorter than its \
                 declaration makes the sanctioned parse fail.\n----- face -----\n{text}\n----- end -----",
                row.n,
                row.bytes,
                text.len() - cur,
            )
        });
        bodies.push(body.to_string());
        cur = end;
    }

    assert_eq!(
        cur,
        text.len(),
        "[{case}] the face must end exactly where its last declared body ends; \
         {} bytes are unaccounted for:\n{:?}",
        text.len() - cur,
        &text[cur..],
    );
    bodies
}

/// One fixture workspace on disk, plus one v3 serve session over it — the same
/// production path the U15 goldens drive (the registry host's line loop,
/// `crate::daemon_door`), so the property is asserted on what a caller
/// actually receives.
pub(crate) fn render_face(doc_name: &str, body: &str, request: &Value) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let docs = dir.path().join("corpus");
    std::fs::create_dir_all(&docs).expect("corpus dir");
    std::fs::write(docs.join(doc_name), body).expect("write fixture");

    let mut input = crate::daemon_door::hello_line(Some("v3"), dir.path());
    input.push_str(&serde_json::to_string(request).expect("request serializes"));
    input.push('\n');

    let frames = crate::daemon_door::serve_frames(&input);
    let read = &frames[1];
    assert_eq!(read["ok"], json!(true), "the read was served: {read}");
    read["body"]["rendered_text"]
        .as_str()
        .expect("rendered_text is a string")
        .to_string()
}
