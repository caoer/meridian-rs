//! The TOON-compact encoder (U15, DECISION 27 · requirements D1 / decision 14 / R1.6):
//! **our own**, over a fixed-shape value model.
//!
//! `toon-format/spec` (v4.1 working draft, 2026-07-26) and `toon-rust` were read as
//! REFERENCE and are not depended on. The render face encodes a handful of
//! statically-known shapes — a header object plus one uniform row table — so a
//! serde-driven general encoder would buy a generality this plane never spends, and what
//! it would cost is a dependency on someone else's quoting law at the exact seam where
//! the goldens pin bytes. The spec's normative rules are reproduced here as code and
//! named at each predicate.
//!
//! # The subset this encoder emits
//! - scalars — `key: value`, exactly one space after the colon (spec §12)
//! - nested objects — `key:` then the object one indent deeper (§8)
//! - arrays of scalars — the inline form, `key[N]: a,b,c` (§9.1)
//! - arrays of objects, **tabular** (§9.3) — `key[N]{f1,f2}:` then one delimiter-joined
//!   row per element, including the **nested-uniform** column groups `f{sub1,sub2}` that
//!   flatten a struct-valued field into sub-columns in place, recursively and unboundedly
//! - arrays that do not classify as tabular — the list form (§9.4)
//! - empty arrays — `key: []`; the legacy `key[0]:` header MUST NOT be emitted
//!
//! # Not emitted
//! The keyed-tabular form (§9.5) and non-comma delimiters (§11) are legal TOON this plane
//! has no shape for. They are absent rather than stubbed: an unexercised branch in an
//! encoder is a byte format nothing pins.
//!
//! # Type fidelity is the quoting law
//! A string is quoted whenever emitting it bare would decode back as something else — a
//! number (including the zero-padded `007` that is a string wearing a number's clothes),
//! a bool, a null, the empty string — or whenever it carries a byte the grammar owns.
//! Quoting is decided per scalar, never per column, so a table where one cell needs
//! quotes does not pay for it in every row. The predicate is exact rather than
//! conservative: over-quoting would be lossless but this face is read by humans (R1.6 —
//! *arrays for machines, TOON for humans*), and a document where every cell wears quotes
//! is the noise the format exists to remove.
//!
//!

use std::fmt::Write as _;

/// The delimiter between tabular cells and between inline array elements.
/// Named once: the quoting predicate and the row writer must agree on it, and
/// two spellings of a comma is how they would drift. Comma is TOON's default
/// document delimiter (§13) — this plane never varies it.
const DELIM: char = ',';

/// The indent one nesting level adds. Spaces only — tabs MUST NOT indent
/// (§12).
const INDENT: &str = "  ";

/// A fixed-shape value: exactly what this plane projects, and no more.
///
/// Objects carry their fields as an ordered `Vec` rather than a map, because
/// the emitted field ORDER is part of the pinned face — a `BTreeMap` would
/// silently alphabetize the columns and a `HashMap` would reorder them per
/// run. The order is the caller's, verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A string. Quoted on emission only when bare bytes would not decode back
    /// to this same string (§ Type fidelity).
    Str(String),
    /// An unsigned integer, always bare.
    Uint(u64),
    /// A boolean, always bare (`true` / `false`).
    Bool(bool),
    /// Null, always bare (`null`).
    Null,
    /// An object: ordered fields.
    Obj(Vec<(String, Value)>),
    /// An array.
    Arr(Vec<Value>),
}

impl Value {
    /// A string field, for call sites that would otherwise spell
    /// `Value::Str(x.to_string())` at every column.
    #[must_use]
    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }

    /// An object from its ordered fields.
    #[must_use]
    pub fn obj<K: Into<String>>(fields: impl IntoIterator<Item = (K, Value)>) -> Value {
        Value::Obj(fields.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }

    /// Whether this value can sit in a tabular cell or an inline array — i.e.
    /// whether it is a primitive. A container cannot: the row grammar is
    /// one-line-per-element, and a container's spelling is more than one line
    /// unless a column group flattens it (§ [`classify`]).
    fn is_primitive(&self) -> bool {
        matches!(
            self,
            Value::Str(_) | Value::Uint(_) | Value::Bool(_) | Value::Null
        )
    }

    /// This value's field, by name — the lookup a tabular row does per column.
    fn field(&self, name: &str) -> Option<&Value> {
        match self {
            Value::Obj(fields) => fields.iter().find(|(k, _)| k == name).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The emitted spelling of a primitive: bare, or quoted when bare would
    /// decode to something else.
    fn cell(&self) -> String {
        match self {
            Value::Str(s) => scalar_str(s),
            Value::Uint(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".to_string(),
            Value::Obj(_) | Value::Arr(_) => unreachable!("cell() is primitives only"),
        }
    }
}

/// Encode a document. The root must be an object — a TOON document is a
/// mapping, and a bare scalar at the root has no key to hang from.
///
/// The result carries no trailing newline: it is a projection a caller splices
/// into a larger face, and the trailing newline is that caller's to add.
#[must_use]
pub fn encode(root: &Value) -> String {
    let mut out = String::new();
    match root {
        Value::Obj(fields) => write_obj(&mut out, fields, 0),
        // A non-object root is a caller bug, not a document. Emit it as the
        // one-value document it is rather than panicking — this plane is
        // panic-free by law (G1).
        Value::Arr(items) => write_array(&mut out, "", items, 0),
        other => {
            let _ = writeln!(out, "{}", other.cell());
        }
    }
    out.truncate(out.trim_end_matches('\n').len());
    out
}

fn write_obj(out: &mut String, fields: &[(String, Value)], depth: usize) {
    for (key, value) in fields {
        write_field(out, key, value, depth);
    }
}

fn write_field(out: &mut String, key: &str, value: &Value, depth: usize) {
    let pad = INDENT.repeat(depth);
    let key = key_str(key);
    match value {
        v if v.is_primitive() => {
            let _ = writeln!(out, "{pad}{key}: {}", v.cell());
        }
        // An empty object is a bare `key:` with no child lines (§8).
        Value::Obj(fields) if fields.is_empty() => {
            let _ = writeln!(out, "{pad}{key}:");
        }
        Value::Obj(fields) => {
            let _ = writeln!(out, "{pad}{key}:");
            write_obj(out, fields, depth + 1);
        }
        Value::Arr(items) => write_array(out, &key, items, depth),
        // unreachable: is_primitive() covered the rest
        _ => {}
    }
}

fn write_array(out: &mut String, key: &str, items: &[Value], depth: usize) {
    let pad = INDENT.repeat(depth);
    let inner = INDENT.repeat(depth + 1);
    let n = items.len();
    let delim = DELIM.to_string();

    // `key: []` — the legacy `key[0]:` header MUST NOT be emitted (§9.1).
    if items.is_empty() {
        let _ = writeln!(out, "{pad}{key}: []");
        return;
    }

    if items.iter().all(Value::is_primitive) {
        let cells: Vec<String> = items.iter().map(Value::cell).collect();
        let _ = writeln!(out, "{pad}{key}[{n}]: {}", cells.join(&delim));
        return;
    }

    if let Some(columns) = tabular_columns(items) {
        let header: Vec<String> = columns.iter().map(Column::header).collect();
        let _ = writeln!(out, "{pad}{key}[{n}]{{{}}}:", header.join(&delim));
        for item in items {
            let mut cells = Vec::new();
            collect_cells(item, &columns, &mut cells);
            let _ = writeln!(out, "{inner}{}", cells.join(&delim));
        }
        return;
    }

    // The list form (§9.4): the array did not classify as tabular, so no
    // header is minted. A header over the UNION of the elements' fields would
    // have to invent a value for every element missing one — and an invented
    // empty cell is indistinguishable from an authored empty string.
    let _ = writeln!(out, "{pad}{key}[{n}]:");
    for item in items {
        match item {
            Value::Str(_) | Value::Uint(_) | Value::Bool(_) | Value::Null => {
                let _ = writeln!(out, "{inner}- {}", item.cell());
            }
            // An empty object as a list item is a bare `-` (§10).
            Value::Obj(fields) if fields.is_empty() => {
                let _ = writeln!(out, "{inner}-");
            }
            Value::Obj(fields) => {
                let mut block = String::new();
                write_obj(&mut block, fields, 0);
                write_list_item(out, &inner, &block);
            }
            Value::Arr(nested) => {
                // `- [2]: 1,2` — the nested array keeps its own header, with
                // the key slot empty because a list item has no key.
                let mut block = String::new();
                write_array(&mut block, "", nested, 0);
                write_list_item(out, &inner, &block);
            }
        }
    }
}

/// Splice one already-rendered block in as a list item: the first line takes
/// the `- ` marker, the rest align under it.
fn write_list_item(out: &mut String, inner: &str, block: &str) {
    for (i, line) in block.lines().enumerate() {
        if i == 0 {
            let _ = writeln!(out, "{inner}- {line}");
        } else {
            let _ = writeln!(out, "{inner}  {line}");
        }
    }
}

/// One tabular column: a primitive column, or a **nested-uniform** group whose
/// struct-valued cells flatten into sub-columns in place (§9.3).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Column {
    Prim(String),
    Group(String, Vec<Column>),
}

impl Column {
    /// The column's spelling in the header: a bare field name, or the
    /// `name{sub,sub}` group form, recursively.
    fn header(&self) -> String {
        match self {
            Column::Prim(name) => key_str(name),
            Column::Group(name, subs) => {
                let inner: Vec<String> = subs.iter().map(Column::header).collect();
                format!("{}{{{}}}", key_str(name), inner.join(&DELIM.to_string()))
            }
        }
    }
}

/// The row's cells, in the depth-first pre-order the header declares (§9.3):
/// a group contributes its leaves in place, never a nested line.
fn collect_cells(item: &Value, columns: &[Column], out: &mut Vec<String>) {
    for column in columns {
        match column {
            Column::Prim(name) => {
                // The classifier proved this field exists and is primitive.
                if let Some(v) = item.field(name) {
                    out.push(v.cell());
                }
            }
            Column::Group(name, subs) => {
                if let Some(v) = item.field(name) {
                    collect_cells(v, subs, out);
                }
            }
        }
    }
}

/// Classify an array into tabular columns, or `None` when it does not qualify.
///
/// Tabular requires every element to be a NON-EMPTY object over the same key
/// set, and every column to be uniform-primitive or nested-uniform (§9.3).
/// The emitted field order is the FIRST element's encounter order; later
/// elements may spell their fields in another order and are read by name, per
/// the spec's "order per object MAY vary".
fn tabular_columns(items: &[Value]) -> Option<Vec<Column>> {
    let first = match items.first()? {
        Value::Obj(fields) if !fields.is_empty() => fields,
        _ => return None,
    };
    let names: Vec<&str> = first.iter().map(|(k, _)| k.as_str()).collect();

    // Same key SET everywhere, and no element is a non-object or empty object.
    for item in items {
        let Value::Obj(fields) = item else {
            return None;
        };
        if fields.len() != names.len() {
            return None;
        }
        if !names.iter().all(|n| item.field(n).is_some()) {
            return None;
        }
    }

    names.iter().map(|name| classify(items, name)).collect()
}

/// Classify one column across every element: primitive, a nested-uniform
/// group, or disqualifying.
///
/// A column mixing a null with objects, or carrying any array or empty object,
/// disqualifies the whole array — the spec names those cases, and each is one
/// where a flat row would have to invent a cell.
fn classify(items: &[Value], name: &str) -> Option<Column> {
    let values: Vec<&Value> = items.iter().filter_map(|i| i.field(name)).collect();
    if values.len() != items.len() {
        return None;
    }
    if values.iter().all(|v| v.is_primitive()) {
        return Some(Column::Prim(name.to_string()));
    }
    // Every value must be a non-empty object over one key set; then recurse.
    let mut sub_names: Option<Vec<String>> = None;
    let mut sub_items: Vec<Value> = Vec::with_capacity(values.len());
    for v in &values {
        let Value::Obj(fields) = v else {
            return None;
        };
        if fields.is_empty() {
            return None;
        }
        match &sub_names {
            None => sub_names = Some(fields.iter().map(|(k, _)| k.clone()).collect()),
            Some(first) => {
                if fields.len() != first.len() || !first.iter().all(|k| v.field(k).is_some()) {
                    return None;
                }
            }
        }
        sub_items.push((*v).clone());
    }
    let sub_names = sub_names?;
    let subs: Vec<Column> = sub_names
        .iter()
        .map(|n| classify(&sub_items, n))
        .collect::<Option<_>>()?;
    Some(Column::Group(name.to_string(), subs))
}

/// A key's emitted spelling (§7.3): bare only when it matches
/// `^[A-Za-z_][A-Za-z0-9_.]*$`, quoted otherwise. The rule is deliberately
/// simpler than the value rule — nothing left of a colon is decoded as a
/// number, so a key never quotes for merely looking like one.
fn key_str(k: &str) -> String {
    let mut chars = k.chars();
    let head_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if head_ok && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') {
        k.to_string()
    } else {
        quoted(k)
    }
}

/// A string scalar's emitted spelling: bare when bare round-trips, quoted when
/// it would not. The trigger list is §7.2, in the spec's order.
#[must_use]
pub fn scalar_str(s: &str) -> String {
    let first = s.chars().next();
    let last = s.chars().last();
    let pad = |c: Option<char>| matches!(c, Some(' ' | '\t'));

    let must_quote = s.is_empty()
        || pad(first)
        || pad(last)
        || matches!(s, "true" | "false" | "null")
        || numeric_like(s)
        || s.chars()
            .any(|c| c == ':' || c == '"' || c == '\\' || c == DELIM || c.is_control())
        || s.chars().any(|c| matches!(c, '[' | ']' | '{' | '}'))
        || matches!(first, Some('-' | '#'));

    if must_quote { quoted(s) } else { s.to_string() }
}

/// The spec's numeric-like predicate (§7.2), over the SPELLING:
/// `^[+-]?[0-9]+(\.[0-9]+)?([eE][+-]?[0-9]+)?$`, ASCII digits only.
///
/// Written as a scanner rather than delegated to `str::parse::<f64>()`, which
/// answers a different question: it accepts `inf`, `NaN`, and the trailing-dot
/// `1.`, and it rejects nothing for having a leading zero. The zero-padded
/// `007` is exactly the case that matters here — a string wearing a number's
/// clothes, which must be quoted or the round trip loses its type.
fn numeric_like(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let digits = |i: &mut usize| {
        let start = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i > start
    };
    if !digits(&mut i) {
        return false;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        if !digits(&mut i) {
            return false;
        }
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        if !digits(&mut i) {
            return false;
        }
    }
    i == b.len()
}

/// The quoted spelling with the escape table (§7.1), first match wins. Any
/// other control character takes the `\uXXXX` form; everything else rides as
/// literal UTF-8, which is what the spec asks encoders to prefer.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Value {
        Value::str(v)
    }

    /// The tabular form is the point of the format: a uniform object array
    /// becomes a header plus one delimiter-joined line per element.
    #[test]
    fn uniform_object_array_renders_tabular() {
        let doc = Value::obj([
            ("path", s("$S/x.md")),
            ("words", Value::Uint(42)),
            (
                "toc",
                Value::Arr(vec![
                    Value::obj([("n", s("1")), ("address", s("Todo"))]),
                    Value::obj([("n", s("2.1")), ("address", s("Notes/Deep"))]),
                ]),
            ),
        ]);
        assert_eq!(
            encode(&doc),
            "path: $S/x.md\nwords: 42\ntoc[2]{n,address}:\n  \"1\",Todo\n  \"2.1\",Notes/Deep"
        );
    }

    /// A struct-valued field is a NESTED-UNIFORM column: it flattens into
    /// sub-columns declared as a group in the header and expanded depth-first
    /// in place in every row — not a nested table, and not a fallback to the
    /// list form. Spec §9.3, Appendix A's `orders` example.
    #[test]
    fn struct_cells_flatten_into_nested_uniform_column_groups() {
        let order = |id: u64, name: &str, country: &str, total: u64| {
            Value::obj([
                ("id", Value::Uint(id)),
                (
                    "customer",
                    Value::obj([("name", s(name)), ("country", s(country))]),
                ),
                ("total", Value::Uint(total)),
            ])
        };
        let doc = Value::obj([(
            "orders",
            Value::Arr(vec![order(1, "Ada", "DK", 99), order(2, "Bob", "UK", 149)]),
        )]);
        assert_eq!(
            encode(&doc),
            "orders[2]{id,customer{name,country},total}:\n  1,Ada,DK,99\n  2,Bob,UK,149"
        );
    }

    /// The recursion has no depth limit: a group inside a group declares and
    /// flattens the same way.
    #[test]
    fn nested_uniform_groups_recurse_without_a_depth_limit() {
        let row = |v: u64| {
            Value::obj([(
                "a",
                Value::obj([("b", Value::obj([("c", Value::Uint(v))]))]),
            )])
        };
        let doc = Value::obj([("rows", Value::Arr(vec![row(1), row(2)]))]);
        assert_eq!(encode(&doc), "rows[2]{a{b{c}}}:\n  1\n  2");
    }

    /// Heterogeneous properties refuse the table and take the list form — no
    /// union header, because a cell invented for a missing field cannot be
    /// told apart from an authored empty one.
    #[test]
    fn heterogeneous_object_array_renders_as_a_list() {
        let doc = Value::obj([(
            "rows",
            Value::Arr(vec![
                Value::obj([("a", Value::Uint(1)), ("b", Value::Uint(2))]),
                Value::obj([("a", Value::Uint(3))]),
            ]),
        )]);
        assert_eq!(encode(&doc), "rows[2]:\n  - a: 1\n    b: 2\n  - a: 3");
    }

    /// The same field SET in another ORDER is still one shape: the header is
    /// the first element's order and later rows are read BY NAME, so the
    /// second element's `b,a` spelling lands in the `a,b` columns rather than
    /// silently transposing its values.
    #[test]
    fn field_order_may_vary_per_element_and_rows_are_read_by_name() {
        let doc = Value::obj([(
            "rows",
            Value::Arr(vec![
                Value::obj([("a", Value::Uint(1)), ("b", Value::Uint(2))]),
                Value::obj([("b", Value::Uint(4)), ("a", Value::Uint(3))]),
            ]),
        )]);
        assert_eq!(encode(&doc), "rows[2]{a,b}:\n  1,2\n  3,4");
    }

    /// A column carrying an array disqualifies the whole table: a flat row has
    /// no cell for it.
    #[test]
    fn an_array_valued_column_disqualifies_the_table() {
        let doc = Value::obj([(
            "rows",
            Value::Arr(vec![Value::obj([
                ("id", Value::Uint(1)),
                ("tags", Value::Arr(vec![s("x"), s("y")])),
            ])]),
        )]);
        assert_eq!(encode(&doc), "rows[1]:\n  - id: 1\n    tags[2]: x,y");
    }

    /// Scalars-only arrays take the inline form; an empty array is `[]` and
    /// never the retired `key[0]:` header.
    #[test]
    fn scalar_arrays_inline_and_empty_arrays_collapse() {
        let doc = Value::obj([
            ("tags", Value::Arr(vec![s("a"), s("b")])),
            ("none", Value::Arr(vec![])),
        ]);
        let out = encode(&doc);
        assert_eq!(out, "tags[2]: a,b\nnone: []");
        assert!(
            !out.contains("[0]"),
            "the legacy empty header is never emitted"
        );
    }

    /// The quoting law, one case per trigger. Each of these decodes back to a
    /// DIFFERENT value if emitted bare — that is the whole predicate.
    #[test]
    fn a_string_is_quoted_exactly_when_bare_would_lie() {
        // number lookalikes, including the zero-padded and exponent spellings
        assert_eq!(scalar_str("007"), "\"007\"");
        assert_eq!(scalar_str("2.1"), "\"2.1\"");
        assert_eq!(scalar_str("1"), "\"1\"");
        assert_eq!(scalar_str("+1"), "\"+1\"");
        assert_eq!(scalar_str("1e-6"), "\"1e-6\"");
        // bool and null lookalikes
        assert_eq!(scalar_str("true"), "\"true\"");
        assert_eq!(scalar_str("null"), "\"null\"");
        // the bytes the grammar owns
        assert_eq!(scalar_str("a,b"), "\"a,b\"");
        assert_eq!(scalar_str("7 items: done"), "\"7 items: done\"");
        assert_eq!(scalar_str("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(scalar_str("a\\b"), "\"a\\\\b\"");
        assert_eq!(scalar_str("one\ntwo"), "\"one\\ntwo\"");
        assert_eq!(scalar_str("a[0]"), "\"a[0]\"");
        assert_eq!(scalar_str("{x}"), "\"{x}\"");
        // the list-marker and comment-line collisions
        assert_eq!(scalar_str("-dash"), "\"-dash\"");
        assert_eq!(scalar_str("#tag"), "\"#tag\"");
        // whitespace the trim would eat, and the empty string
        assert_eq!(scalar_str(" pad "), "\" pad \"");
        assert_eq!(scalar_str("\tpad"), "\"\\tpad\"");
        assert_eq!(scalar_str(""), "\"\"");
        // any other control byte takes the \uXXXX form
        assert_eq!(scalar_str("a\u{1}b"), "\"a\\u0001b\"");
    }

    /// And bare exactly when bare is safe: an ordinary human address, a
    /// three-part ordinal, an anchor. This is the half that keeps the face
    /// readable — an encoder that quotes everything is lossless and useless.
    #[test]
    fn an_ordinary_string_stays_bare() {
        assert_eq!(
            scalar_str("Notes/Slash-Title-Here"),
            "Notes/Slash-Title-Here"
        );
        assert_eq!(scalar_str("^goal"), "^goal");
        assert_eq!(scalar_str("1.2.3"), "1.2.3");
        assert_eq!(scalar_str("v1"), "v1");
        assert_eq!(scalar_str("e3"), "e3");
        assert_eq!(scalar_str("1."), "1.");
    }

    /// A key is never quoted for looking like a number — nothing left of a
    /// colon is decoded as one — but the bare form is the spec's identifier
    /// shape, so a key that leaves it quotes.
    #[test]
    fn keys_take_the_identifier_rule_not_the_value_rule() {
        assert_eq!(key_str("file_rev"), "file_rev");
        assert_eq!(key_str("a.b"), "a.b");
        assert_eq!(key_str("1"), "\"1\"");
        assert_eq!(key_str("my-key"), "\"my-key\"");
        assert_eq!(key_str("a,b"), "\"a,b\"");
        assert_eq!(key_str(""), "\"\"");
    }

    /// Nested objects indent by two, an empty object is a bare key, and the
    /// document carries no trailing newline.
    #[test]
    fn nested_objects_indent_and_the_document_does_not_end_in_newline() {
        let doc = Value::obj([
            ("top", Value::Uint(1)),
            (
                "inner",
                Value::obj([("a", s("x")), ("deeper", Value::obj([("b", Value::Null)]))]),
            ),
            ("empty", Value::Obj(vec![])),
        ]);
        let out = encode(&doc);
        assert_eq!(
            out,
            "top: 1\ninner:\n  a: x\n  deeper:\n    b: null\nempty:"
        );
        assert!(!out.ends_with('\n'));
    }

    /// A nested array inside a list item keeps its own inline header (§9.4's
    /// `- [2]: 1,2`) — the key slot is empty because a list item has no key.
    #[test]
    fn a_nested_array_list_item_keeps_its_own_header() {
        let doc = Value::obj([(
            "pairs",
            Value::Arr(vec![
                Value::Arr(vec![Value::Uint(1), Value::Uint(2)]),
                Value::Arr(vec![Value::Uint(3), Value::Uint(4)]),
            ]),
        )]);
        assert_eq!(encode(&doc), "pairs[2]:\n  - [2]: 1,2\n  - [2]: 3,4");
    }
}
