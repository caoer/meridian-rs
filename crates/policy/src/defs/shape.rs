//! The shape-language evaluator — Go `internal/defs/shape.go` ported verbatim:
//! nine closed shapes, four guards, three stamps (schema-v2 §1.2, R6). The set
//! is enumerated here and nowhere else.

use super::fm::FmValue;

pub(super) const SHAPE_LINE: &str = "line";
pub(super) const SHAPE_TEXT: &str = "text";
pub(super) const SHAPE_ISO: &str = "iso";
pub(super) const SHAPE_INT: &str = "int";
pub(super) const SHAPE_BOOL: &str = "bool";
pub(super) const SHAPE_REF: &str = "ref";
pub(super) const SHAPE_LIST_LINE: &str = "list(line)";
pub(super) const SHAPE_LIST_REF: &str = "list(ref)";
pub(super) const SHAPE_LIST_ISO: &str = "list(iso)";

pub(super) const GUARD_ACTOR_NOT_OWNER: &str = "actor-not-owner";
pub(super) const GUARD_APPEND_ONLY: &str = "append-only";
const GUARD_SECTION_PREFIX: &str = "section-non-empty(";
const GUARD_REQUIRES_PREFIX: &str = "requires(";

/// `ValidShape`: one of the nine closed shapes.
pub(super) fn valid_shape(s: &str) -> bool {
    matches!(
        s,
        SHAPE_LINE
            | SHAPE_TEXT
            | SHAPE_ISO
            | SHAPE_INT
            | SHAPE_BOOL
            | SHAPE_REF
            | SHAPE_LIST_LINE
            | SHAPE_LIST_REF
            | SHAPE_LIST_ISO
    )
}

/// The closed stamp set ("" allowed = unset).
pub(super) fn valid_stamp(s: &str) -> bool {
    matches!(s, "" | "create" | "touch" | "close")
}

/// `ParseGuard`: (kind, arg, ok) against the closed guard vocabulary.
pub(super) fn parse_guard(g: &str) -> Option<(String, String)> {
    if g == GUARD_ACTOR_NOT_OWNER || g == GUARD_APPEND_ONLY {
        return Some((g.to_string(), String::new()));
    }
    if let Some(arg) = g
        .strip_prefix(GUARD_SECTION_PREFIX)
        .and_then(|s| s.strip_suffix(')'))
    {
        if arg.is_empty() {
            return None;
        }
        return Some(("section-non-empty".to_string(), arg.to_string()));
    }
    if let Some(arg) = g
        .strip_prefix(GUARD_REQUIRES_PREFIX)
        .and_then(|s| s.strip_suffix(')'))
    {
        if arg.is_empty() {
            return None;
        }
        return Some(("requires".to_string(), arg.to_string()));
    }
    None
}

/// `CheckShape`: does a decoded frontmatter value satisfy the shape? A Null value
/// is ABSENT, not a shape violation — callers filter it first (Go law).
pub(super) fn check_shape(shape: &str, v: &FmValue) -> bool {
    match shape {
        SHAPE_LINE => matches!(v, FmValue::Str(s) if !s.contains('\n')),
        SHAPE_TEXT => matches!(v, FmValue::Str(_)),
        SHAPE_ISO => is_iso(v),
        SHAPE_INT => matches!(v, FmValue::Int(_)),
        SHAPE_BOOL => matches!(v, FmValue::Bool(_)),
        SHAPE_REF => matches!(v, FmValue::Str(s) if is_wikilink(s)),
        SHAPE_LIST_LINE | SHAPE_LIST_REF | SHAPE_LIST_ISO => {
            let FmValue::List(items) = v else {
                return false;
            };
            let inner = &shape["list(".len()..shape.len() - 1];
            items.iter().all(|it| check_shape(inner, it))
        }
        _ => false,
    }
}

/// Go `isISO`: a yaml-decoded time.Time, or a string in one of the three
/// accepted layouts (RFC3339, naive seconds, naive minutes).
fn is_iso(v: &FmValue) -> bool {
    match v {
        FmValue::Timestamp(_) => true,
        FmValue::Str(s) => parses_iso_layout(s),
        _ => false,
    }
}

/// The three Go layouts: RFC3339 (`2006-01-02T15:04:05Z07:00`),
/// `2006-01-02T15:04:05` (seconds-exact), `2006-01-02T15:04`.
fn parses_iso_layout(s: &str) -> bool {
    let b = s.as_bytes();
    let date_ok = b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit);
    if !date_ok || b.len() < 16 || b[10] != b'T' {
        return false;
    }
    let two = |i: usize| b.len() >= i + 2 && b[i].is_ascii_digit() && b[i + 1].is_ascii_digit();
    if !(two(11) && b.len() >= 16 && b[13] == b':' && two(14)) {
        return false;
    }
    match b.len() {
        16 => true,                     // 2006-01-02T15:04
        19 => b[16] == b':' && two(17), // 2006-01-02T15:04:05
        _ => {
            // RFC3339: seconds + zone (Z or ±hh:mm), optional fraction.
            if b.len() < 20 || b[16] != b':' || !two(17) {
                return false;
            }
            let mut i = 19;
            if b[i] == b'.' {
                i += 1;
                let frac_start = i;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                if i == frac_start {
                    return false;
                }
            }
            if i >= b.len() {
                return false;
            }
            match b[i] {
                b'Z' => i + 1 == b.len(),
                b'+' | b'-' => b.len() == i + 6 && two(i + 1) && b[i + 3] == b':' && two(i + 4),
                _ => false,
            }
        }
    }
}

/// Go `reWikilink`: `^\[\[.+\]\]$`.
fn is_wikilink(s: &str) -> bool {
    s.len() > 4 && s.starts_with("[[") && s.ends_with("]]")
}

/// `IsNested`: the substrate-law class — a mapping anywhere, or list-in-list.
/// The fm parser already folds those into `FmValue::Nested`.
pub(super) fn is_nested(v: &FmValue) -> bool {
    matches!(v, FmValue::Nested)
}
