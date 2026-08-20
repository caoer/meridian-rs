//! Effect-shim FD protocol (U6a, ruling 1, review S6) — the ONLY channel for
//! governed tree change from a bash block. Bash writes md.* descriptors as
//! NDJSON lines on an inherited FD; the parent parses, validates, and hands
//! them to the executor. Truncation or malformation fails the whole batch (S6).
//!
//! # Wire grammar
//! One JSON object per line. Required keys depend on `op` (the effect kind);
//! `md.create` also admits the optional `base` (the targeting axis — ZT ruling
//! 2026-08-19 #2, boundary as data). Unknown ops and unknown keys refuse.
//! Empty stream is a valid zero-effect batch.
//!
//! # Authority
//! Descriptors carry no capability of their own — the executor validates
//! against the block's [`Authority`] at the choke point. Bash is always
//! [`Authority::Unsandboxed`]; caps do not apply (`docs/laws.md` amendment).

use std::collections::BTreeMap;

use effects::{ArgValue, Effect, EffectKind, Provenance};

/// Hard cap on one record's payload (resource bound, C4 spirit).
pub const MAX_RECORD_BYTES: usize = 1 << 20;
/// Hard cap on the record count of one stream.
pub const MAX_RECORDS: usize = 4096;

/// The captured shim-fd stream: the raw bytes plus the capture-side overflow
/// fact (the reader stopped storing past its cap). Built by `exec`, consumed
/// by [`parse`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShimStream {
    /// The bytes read off the shim fd, in emission order.
    pub bytes: Vec<u8>,
    /// The reader hit its storage cap — the stream is incomplete BY
    /// CONSTRUCTION and [`parse`] refuses it.
    pub overflowed: bool,
}

/// One parsed shim descriptor — inert data, the bash-side twin of the kernel
/// constructors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShimDescriptor {
    /// `md.set_field` — set a frontmatter field.
    SetField {
        /// The frontmatter key.
        field: String,
        /// The value to set.
        value: String,
    },
    /// `md.append_section` — append content to a section.
    AppendSection {
        /// The governing heading text.
        section: String,
        /// The content to append.
        content: String,
    },
    /// `md.create` — birth a file through the create door (the declared-task
    /// birth cap).
    Create {
        /// The RELATIVE landing coordinate as the block declared it — the
        /// string a capability glob judges. It admits no rooted spelling;
        /// targeting rides `base`.
        path: String,
        /// The new file's whole bytes.
        body: String,
        /// The optional resolution base the birth lands under — a rooted
        /// `root:rel` ref naming this run's own workspace, or a confined
        /// workspace-relative directory. Absent, the caller's ambient
        /// directory is the default base; absent both, the path lands
        /// workspace-root-relative. Identical semantics to starlark
        /// `create(base = …)`.
        base: Option<String>,
    },
}

/// Why the stream refused. EVERY variant fails the whole phase-2 batch —
/// there is no partial acceptance of a shim stream (S6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShimError {
    /// The capture overflowed the reader's storage cap.
    Overflow,
    /// The stream ended inside a frame (mid-prefix or mid-payload).
    Truncated {
        /// Byte offset the unfinished frame starts at.
        at: usize,
    },
    /// A frame prefix is not `<decimal>:` and not the `end:` trailer.
    BadFrame {
        /// Byte offset of the bad frame.
        at: usize,
    },
    /// A record's declared length exceeds [`MAX_RECORD_BYTES`].
    RecordTooLarge {
        /// The declared payload length.
        len: usize,
    },
    /// The stream declares more than [`MAX_RECORDS`] records.
    TooManyRecords,
    /// A payload is not the expected JSON object shape.
    Malformed {
        /// Zero-based record index.
        index: usize,
        /// What was wrong.
        reason: String,
    },
    /// A payload names an op the shim does not admit (the shim surface is
    /// `md.set_field` | `md.append_section` | `md.create` — ruling 1, plus the
    /// birth lane).
    UnknownOp {
        /// Zero-based record index.
        index: usize,
        /// The op named.
        op: String,
    },
    /// The stream has records but never states the `end:<count>` trailer.
    MissingTrailer,
    /// The trailer count disagrees with the records present.
    CountMismatch {
        /// The trailer's declared count.
        declared: usize,
        /// Records actually parsed.
        actual: usize,
    },
    /// Bytes follow the trailer — the trailer must be the final bytes.
    TrailingData {
        /// Byte offset of the first excess byte.
        at: usize,
    },
}

impl std::fmt::Display for ShimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShimError::Overflow => write!(f, "shim stream overflowed the capture cap"),
            ShimError::Truncated { at } => {
                write!(f, "shim stream truncated inside a frame at byte {at}")
            }
            ShimError::BadFrame { at } => write!(f, "shim stream: bad frame at byte {at}"),
            ShimError::RecordTooLarge { len } => write!(
                f,
                "shim record of {len} bytes exceeds the {MAX_RECORD_BYTES}-byte cap"
            ),
            ShimError::TooManyRecords => {
                write!(f, "shim stream exceeds the {MAX_RECORDS}-record cap")
            }
            ShimError::Malformed { index, reason } => {
                write!(f, "shim record {index} malformed: {reason}")
            }
            ShimError::UnknownOp { index, op } => write!(
                f,
                "shim record {index} names op '{op}' (the shim admits md.set_field | md.append_section | md.create)"
            ),
            ShimError::MissingTrailer => {
                write!(f, "shim stream ended without the end:<count> trailer")
            }
            ShimError::CountMismatch { declared, actual } => write!(
                f,
                "shim trailer declares {declared} records, stream carries {actual}"
            ),
            ShimError::TrailingData { at } => {
                write!(f, "shim stream continues after the trailer at byte {at}")
            }
        }
    }
}

impl std::error::Error for ShimError {}

/// Parse a captured shim stream into its descriptors, refusing on ANY framing
/// or shape deviation (S6 — the whole phase-2 batch fails closed).
///
/// # Errors
/// [`ShimError`] — the stream is refused as a whole; there is no partial
/// descriptor list.
pub fn parse(stream: &ShimStream) -> Result<Vec<ShimDescriptor>, ShimError> {
    if stream.overflowed {
        return Err(ShimError::Overflow);
    }
    let bytes = &stream.bytes;
    if bytes.is_empty() {
        return Ok(Vec::new());
    }

    let mut descriptors = Vec::new();
    let mut pos = 0usize;
    let mut trailer_count: Option<usize> = None;

    while pos < bytes.len() {
        let frame_start = pos;
        let Some(colon_rel) = bytes[pos..].iter().position(|&b| b == b':') else {
            // The stream ends inside a frame prefix — cut mid-frame.
            return Err(ShimError::Truncated { at: frame_start });
        };
        let prefix = &bytes[pos..pos + colon_rel];
        pos += colon_rel + 1;

        if prefix == b"end" {
            let Some(nl_rel) = bytes[pos..].iter().position(|&b| b == b'\n') else {
                return Err(ShimError::Truncated { at: frame_start });
            };
            let count = parse_decimal(&bytes[pos..pos + nl_rel])
                .ok_or(ShimError::BadFrame { at: frame_start })?;
            pos += nl_rel + 1;
            if pos != bytes.len() {
                return Err(ShimError::TrailingData { at: pos });
            }
            trailer_count = Some(count);
            break;
        }

        let len = parse_decimal(prefix).ok_or(ShimError::BadFrame { at: frame_start })?;
        if len > MAX_RECORD_BYTES {
            return Err(ShimError::RecordTooLarge { len });
        }
        if descriptors.len() == MAX_RECORDS {
            return Err(ShimError::TooManyRecords);
        }
        // Payload + its terminating newline must both be present in full.
        if pos + len + 1 > bytes.len() {
            return Err(ShimError::Truncated { at: frame_start });
        }
        let payload = &bytes[pos..pos + len];
        if bytes[pos + len] != b'\n' {
            // The declared length does not land on the record terminator.
            return Err(ShimError::BadFrame { at: frame_start });
        }
        pos += len + 1;
        descriptors.push(parse_payload(descriptors.len(), payload)?);
    }

    match trailer_count {
        None => Err(ShimError::MissingTrailer),
        Some(declared) if declared != descriptors.len() => Err(ShimError::CountMismatch {
            declared,
            actual: descriptors.len(),
        }),
        Some(_) => Ok(descriptors),
    }
}

/// Convert parsed descriptors into run-plane [`Effect`]s for the executor:
/// `rule_id` = the task, `seq` = emission index, `depth` = 0, provenance =
/// [`Provenance::Run`] with the caller-supplied invocation identity and the
/// root the step ran against (`root_after_phase1` on the bash path).
#[must_use]
pub fn to_effects(
    descriptors: &[ShimDescriptor],
    task: &str,
    invocation_id: &str,
    root_at_eval: &str,
) -> Vec<Effect> {
    let mut seq: u32 = 0;
    descriptors
        .iter()
        .map(|d| {
            let (kind, args) = match d {
                ShimDescriptor::SetField { field, value } => (
                    EffectKind::SetField,
                    BTreeMap::from([
                        ("field".to_owned(), ArgValue::Str(field.clone())),
                        ("value".to_owned(), ArgValue::Str(value.clone())),
                    ]),
                ),
                ShimDescriptor::AppendSection { section, content } => (
                    EffectKind::AppendSection,
                    BTreeMap::from([
                        ("section".to_owned(), ArgValue::Str(section.clone())),
                        ("content".to_owned(), ArgValue::Str(content.clone())),
                    ]),
                ),
                ShimDescriptor::Create { path, body, base } => {
                    let mut args = BTreeMap::from([
                        ("path".to_owned(), ArgValue::Str(path.clone())),
                        ("body".to_owned(), ArgValue::Str(body.clone())),
                    ]);
                    // Present ONLY when the block declared it — an absent key
                    // and an empty base are different facts to the resolver
                    // (absent = fall back to ambient; empty = refuse).
                    if let Some(base) = base {
                        args.insert("base".to_owned(), ArgValue::Str(base.clone()));
                    }
                    (EffectKind::Create, args)
                }
            };
            let effect = Effect {
                kind,
                rule_id: task.to_owned(),
                seq,
                depth: 0,
                provenance: Provenance::Run {
                    invocation_id: invocation_id.to_owned(),
                    root_at_eval: root_at_eval.to_owned(),
                },
                args,
            };
            seq += 1;
            effect
        })
        .collect()
}

/// Strict ASCII-decimal parse: nonempty, digits only, no sign, no leading `+`.
fn parse_decimal(bytes: &[u8]) -> Option<usize> {
    if bytes.is_empty() || bytes.len() > 12 || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

/// Parse one record payload: a JSON object with an `op` tag and ONLY that
/// op's keys — an extra key is a refusal, not a warning (strict by hand:
/// serde's internally-tagged enums cannot deny unknown fields). A required key
/// must be present and a string; an optional key must be a string when
/// present.
fn parse_payload(index: usize, payload: &[u8]) -> Result<ShimDescriptor, ShimError> {
    let malformed = |reason: String| ShimError::Malformed { index, reason };
    let text =
        std::str::from_utf8(payload).map_err(|_| malformed("payload is not UTF-8".to_owned()))?;
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| malformed(format!("not JSON: {e}")))?;
    let serde_json::Value::Object(obj) = value else {
        return Err(malformed("payload is not a JSON object".to_owned()));
    };
    let op = match obj.get("op") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(_) => return Err(malformed("'op' is not a string".to_owned())),
        None => return Err(malformed("missing 'op'".to_owned())),
    };
    // The op's whole key surface: required first, then optional.
    let (required, optional): (&[&str], &[&str]) = match op.as_str() {
        "md.set_field" => (&["field", "value"], &[]),
        "md.append_section" => (&["section", "content"], &[]),
        // `base` is the targeting axis, optional exactly as in starlark
        // `create(base = …)` — the two facts never ride one glued string.
        "md.create" => (&["path", "body"], &["base"]),
        _ => return Err(ShimError::UnknownOp { index, op }),
    };
    for key in obj.keys() {
        if key != "op" && !required.contains(&key.as_str()) && !optional.contains(&key.as_str()) {
            return Err(malformed(format!("unknown key '{key}' for {op}")));
        }
    }
    let req = |key: &str| -> Result<String, ShimError> {
        match obj.get(key) {
            Some(serde_json::Value::String(s)) => Ok(s.clone()),
            Some(_) => Err(malformed(format!("'{key}' is not a string"))),
            None => Err(malformed(format!("missing '{key}'"))),
        }
    };
    let opt = |key: &str| -> Result<Option<String>, ShimError> {
        match obj.get(key) {
            Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(malformed(format!("'{key}' is not a string"))),
            None => Ok(None),
        }
    };
    Ok(match op.as_str() {
        "md.set_field" => ShimDescriptor::SetField {
            field: req("field")?,
            value: req("value")?,
        },
        "md.append_section" => ShimDescriptor::AppendSection {
            section: req("section")?,
            content: req("content")?,
        },
        "md.create" => ShimDescriptor::Create {
            path: req("path")?,
            body: req("body")?,
            base: opt("base")?,
        },
        // Unreachable — the key-surface match above refused every other op.
        // Spelled out so a new op added there but not here fails CLOSED
        // instead of silently building a birth.
        _ => {
            return Err(ShimError::UnknownOp {
                index,
                op: op.clone(),
            });
        }
    })
}
