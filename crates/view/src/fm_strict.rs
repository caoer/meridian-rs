//! The strict-YAML verdict on a page's frontmatter — the `frontmatter_fault`
//! relation's one input.
//!
//! The engine reads frontmatter with its own lenient scanner (`model`, serde-free
//! by Law 1): `description: a: b` and `description: "a: b"` both project `a: b`.
//! A strict YAML parser (Obsidian's, for one) refuses the first, and a page whose
//! frontmatter it refuses carries no properties in that reader. No query over
//! the projected values can tell the two apart, so the verdict is computed here,
//! by the same pinned `serde_yaml` the `.base` parse uses (`base.rs`), beside
//! its only consumer.

/// One page's strict-YAML refusal: the FILE line (1-based) the parser points
/// at, and the parser's own message.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Fault {
    pub(crate) line: u64,
    pub(crate) error: String,
}

/// Judge the frontmatter block `fenced` (the node's fence-to-fence span) that
/// opens on FILE line `open_line` (1-based). `None` when the interior parses as
/// a YAML mapping, or is empty.
///
/// A root that parses but is not a mapping (a bare scalar, a sequence) is a
/// fault too: it carries no properties for any reader.
pub(crate) fn judge(fenced: &str, open_line: u64) -> Option<Fault> {
    let mut lines: Vec<&str> = fenced.lines().collect();
    if !lines.is_empty() {
        lines.remove(0); // the opening `---`
    }
    if lines.last().is_some_and(|l| l.trim_end() == "---") {
        lines.pop(); // the closing `---`
    }
    let interior = lines.join("\n");
    let first = open_line + 1;
    let line_count = u64::try_from(lines.len().max(1)).unwrap_or(u64::MAX);
    match serde_yaml::from_str::<serde_yaml::Value>(&interior) {
        Ok(serde_yaml::Value::Mapping(_) | serde_yaml::Value::Null) => None,
        Ok(_) => Some(Fault {
            line: first,
            error: "the frontmatter is not a YAML mapping".to_owned(),
        }),
        Err(e) => {
            // serde_yaml locations are 1-based over the handed slice, and an
            // unclosed construct is reported one line past it; clamp into the
            // block so the row points at a line the author can see.
            let at = e
                .location()
                .map_or(1, |loc| u64::try_from(loc.line()).unwrap_or(u64::MAX));
            Some(Fault {
                line: first + at.clamp(1, line_count) - 1,
                error: e.to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::judge;

    fn fenced(interior: &str) -> String {
        format!("---\n{interior}---\n")
    }

    #[test]
    fn a_quoted_colon_value_and_flow_tags_are_clean() {
        assert_eq!(judge(&fenced("description: \"a: b\"\n"), 1), None);
        assert_eq!(
            judge(&fenced("tags: [type/task, a]\nstatus: Todo\n"), 1),
            None
        );
        assert_eq!(judge(&fenced(""), 1), None);
    }

    #[test]
    fn the_shapes_a_strict_parser_refuses_are_faults_on_their_file_line() {
        for (interior, line) in [
            ("type: note\ndescription: a: b\n", 3),
            ("title: *x\n", 2),
            ("type: note\nv: >=2.1\n", 3),
        ] {
            let fault = judge(&fenced(interior), 1)
                .unwrap_or_else(|| panic!("strict YAML refuses {interior:?}"));
            assert_eq!(fault.line, line, "{interior:?}: {}", fault.error);
            assert!(!fault.error.is_empty());
        }
    }

    #[test]
    fn a_non_mapping_root_is_a_fault() {
        let fault = judge(&fenced("just a sentence\n"), 1).expect("a scalar root");
        assert_eq!(fault.line, 2);
    }
}
