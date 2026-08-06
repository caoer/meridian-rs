//! The STORED plane of a cross-root address — the `obsidian://` URI.
//!
//! The agent plane is `[root:]path[#selector][@fp]` ([`crate::Addr`]); the stored plane
//! is what disk carries for a CROSS-ROOT ref, because a root-prefixed wikilink is
//! unresolvable garbage to Obsidian. `put` translates one into the other; `read`
//! translates back.
//!
//! The grammar reproduces the shipped `md encode` canon: `obsidian://open` (no
//! fragment) or `advanced-uri` (heading/^block); strict decode — non-canonical
//! encoding is flagged, never normalized.
//!
//! Stricter than `md` on decode: where the Go canon warns and exits 0, the engine
//! REFUSES — a stored URI is read back into an address a pin is measured against,
//! so "resolving to something plausible" is the wrong-success shape. Loud, with
//! the canonical spelling named.

use core::fmt;

use crate::confined;

/// The scheme every stored cross-root form opens with.
pub const OBSIDIAN_SCHEME: &str = "obsidian://";

/// The canon's page-grain action — no fragment.
pub const ACTION_OPEN: &str = "open";

/// The canon's selector-grain action — a heading or a block ref.
pub const ACTION_ADVANCED: &str = "advanced-uri";

/// A cross-root address in its STORED spelling, parsed.
///
/// Carries the vault name, never a device-local vault id: vault ids are not
/// portable, so a stored id would make one document's link machine-dependent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRef {
    /// The Obsidian vault name the URI names.
    pub vault: String,
    /// The vault-relative path, percent-DECODED.
    pub path: String,
    /// The selector, in the agent plane's own spelling (`^id` keeps its caret).
    /// `None` is page grain.
    pub selector: Option<String>,
}

/// Why a stored form refused to encode or decode — the closed set.
///
/// Every variant carries the offending text; the refusal is read by a human
/// who must hand-fix a URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredError {
    /// The text does not open with [`OBSIDIAN_SCHEME`] — not a stored form at
    /// all. The one variant callers routinely branch on rather than report.
    NotStoredForm,
    /// The action segment is neither [`ACTION_OPEN`] nor [`ACTION_ADVANCED`].
    UnknownAction {
        /// The action as spelled, verbatim.
        found: String,
    },
    /// The parameter sequence is not the canon's for this action.
    BadParams {
        /// The action the URI named.
        action: String,
        /// The parameter names found, in order, joined by `&`.
        found: String,
        /// The canonical sequence for that action.
        expected: String,
    },
    /// A parameter value is percent-encoded differently from the canon.
    /// **Flagged, never normalized** — the canonical spelling rides along so the
    /// fix is one edit.
    NonCanonicalEncoding {
        /// The parameter name.
        param: String,
        /// Its value, verbatim as stored.
        found: String,
        /// The value the canon would have written.
        canonical: String,
    },
    /// A parameter carries an empty value, or the selector is present but
    /// empty. An empty selector has no stored spelling and is REFUSED rather
    /// than folded into page grain — folding would lose the `#` and break the
    /// round trip silently.
    EmptyValue {
        /// The parameter (or `selector`) that was empty.
        param: String,
    },
    /// The decoded path is not a confined corpus path ([`confined`]).
    BadPath {
        /// The offending path, decoded.
        found: String,
    },
}

impl fmt::Display for StoredError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoredError::NotStoredForm => f.write_str(
                "refused: not an `obsidian://` stored form — a cross-root address is stored as \
                 the URI the OS dispatches; see [[address-grammar]].",
            ),
            StoredError::UnknownAction { found } => write!(
                f,
                "refused: '{found}' is not a canonical obsidian action — the canon emits \
                 `open` (page grain) or `advanced-uri` (heading / block). \
                 Fix: re-mint the link with `mrd put` rather than hand-editing it; \
                 see [[address-grammar]]."
            ),
            StoredError::BadParams {
                action,
                found,
                expected,
            } => write!(
                f,
                "refused: the `{action}` stored form carries `{found}` — the canonical sequence \
                 is `{expected}`. The engine reads the canonical form only, so a hand-edited URI \
                 fails here rather than resolving to something plausible. \
                 Fix: re-mint the link with `mrd put`; see [[address-grammar]]."
            ),
            StoredError::NonCanonicalEncoding {
                param,
                found,
                canonical,
            } => write!(
                f,
                "refused: the `{param}` parameter is not canon-encoded — stored as '{found}', \
                 canonical is '{canonical}'. Flagged, never silently normalized: this URI is read \
                 back into the address a pin is measured against. \
                 Fix: write the canonical spelling, or re-mint with `mrd put`; \
                 see [[address-grammar]]."
            ),
            StoredError::EmptyValue { param } => write!(
                f,
                "refused: the stored form's `{param}` is empty — an empty {param} addresses \
                 nothing and has no stored spelling. \
                 Fix: name the target, or drop the fragment entirely; see [[address-grammar]]."
            ),
            StoredError::BadPath { found } => write!(
                f,
                "refused: '{found}' is not a corpus path — a stored form names a confined, \
                 root-relative path with no `..`, no leading `/` and no root separator in its \
                 head. Fix: re-mint the link with `mrd put`; see [[address-grammar]]."
            ),
        }
    }
}

impl std::error::Error for StoredError {}

/// Does this text open with the stored form's scheme? The cheap pre-filter every
/// positional scan applies before paying for a parse.
#[must_use]
pub fn is_stored_uri(text: &str) -> bool {
    text.starts_with(OBSIDIAN_SCHEME)
}

/// The canon's percent-encoding: encode `%`, `&`, `=`, `?`, `#`, `+`, space,
/// every control byte and every non-ASCII byte as uppercase `%XX`; leave every
/// other ASCII byte literal — `/` included, so a stored path reads as a path.
#[must_use]
pub fn encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        let literal = b.is_ascii()
            && b > 0x20
            && b != 0x7F
            && !matches!(b, b'%' | b'&' | b'=' | b'?' | b'#' | b'+');
        if literal {
            out.push(b as char);
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0F) as usize] as char);
        }
    }
    out
}

/// Percent-DECODE one component. `None` when a `%` is not followed by two hex
/// digits, or when the decoded bytes are not UTF-8 — a malformed escape is
/// never guessed past.
#[must_use]
pub fn decode_component(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let hex = core::str::from_utf8(hex).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Mint the stored form of a cross-root address.
///
/// `selector` is the agent plane's own spelling — `^id` for a block ref, the
/// heading text otherwise, `None` for page grain. The `@fp` decoration is not
/// a parameter: it is minted on read and never stored (§ 4.4).
///
/// # Errors
///
/// [`StoredError::EmptyValue`] for an empty vault or an empty selector,
/// [`StoredError::BadPath`] for a path that is not a confined corpus path.
pub fn encode(vault: &str, path: &str, selector: Option<&str>) -> Result<String, StoredError> {
    if vault.is_empty() {
        return Err(StoredError::EmptyValue {
            param: "vault".to_string(),
        });
    }
    if !confined(path) {
        return Err(StoredError::BadPath {
            found: path.to_string(),
        });
    }
    let vault_enc = encode_component(vault);
    let path_enc = encode_component(path);
    match selector {
        None => Ok(format!(
            "{OBSIDIAN_SCHEME}{ACTION_OPEN}?vault={vault_enc}&file={path_enc}"
        )),
        Some("") => Err(StoredError::EmptyValue {
            param: "selector".to_string(),
        }),
        Some(sel) => {
            let (key, value) = match sel.strip_prefix('^') {
                Some(block) => ("block", block),
                None => ("heading", sel),
            };
            if value.is_empty() {
                return Err(StoredError::EmptyValue {
                    param: "selector".to_string(),
                });
            }
            Ok(format!(
                "{OBSIDIAN_SCHEME}{ACTION_ADVANCED}?vault={vault_enc}&filepath={path_enc}&{key}={}",
                encode_component(value)
            ))
        }
    }
}

/// Read a stored form back into its parts — strictly.
///
/// The parameter sequence must be the canon's and every value encoded the way
/// [`encode`] writes it. Anything else refuses, naming the canonical spelling:
/// this URI is read back into the address a pin is measured against.
///
/// # Errors
///
/// The closed set of [`StoredError`]. [`StoredError::NotStoredForm`] when the
/// text is not a stored form at all — callers branch on that one.
pub fn decode(uri: &str) -> Result<StoredRef, StoredError> {
    let rest = uri
        .strip_prefix(OBSIDIAN_SCHEME)
        .ok_or(StoredError::NotStoredForm)?;
    let (action, query) = rest.split_once('?').unwrap_or((rest, ""));
    if action != ACTION_OPEN && action != ACTION_ADVANCED {
        return Err(StoredError::UnknownAction {
            found: action.to_string(),
        });
    }
    let params: Vec<(&str, &str)> = if query.is_empty() {
        Vec::new()
    } else {
        query
            .split('&')
            .map(|p| p.split_once('=').unwrap_or((p, "")))
            .collect()
    };
    let names: Vec<&str> = params.iter().map(|(k, _)| *k).collect();

    let bad_params = |expected: &str| StoredError::BadParams {
        action: action.to_string(),
        found: names.join("&"),
        expected: expected.to_string(),
    };

    let (path_key, selector) = match (action, names.as_slice()) {
        (ACTION_OPEN, ["vault", "file"]) => ("file", None),
        (ACTION_ADVANCED, ["vault", "filepath", key @ ("heading" | "block")]) => {
            ("filepath", Some(*key))
        }
        (ACTION_OPEN, _) => return Err(bad_params("vault&file")),
        _ => return Err(bad_params("vault&filepath&(heading|block)")),
    };

    let value_of = |key: &str| -> Result<String, StoredError> {
        let raw = params
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
            .unwrap_or_default();
        if raw.is_empty() {
            return Err(StoredError::EmptyValue {
                param: key.to_string(),
            });
        }
        let decoded = decode_component(raw).ok_or_else(|| StoredError::NonCanonicalEncoding {
            param: key.to_string(),
            found: raw.to_string(),
            canonical: encode_component(raw),
        })?;
        // Canon check: `%2F` and `/` decode to the same byte, so only this
        // re-encode comparison can tell engine-minted from hand-edited.
        if encode_component(&decoded) != raw {
            return Err(StoredError::NonCanonicalEncoding {
                param: key.to_string(),
                found: raw.to_string(),
                canonical: encode_component(&decoded),
            });
        }
        Ok(decoded)
    };

    let vault = value_of("vault")?;
    let path = value_of(path_key)?;
    if !confined(&path) {
        return Err(StoredError::BadPath { found: path });
    }
    let selector = match selector {
        None => None,
        Some(key) => {
            let value = value_of(key)?;
            Some(if key == "block" {
                format!("^{value}")
            } else {
                value
            })
        }
    };
    Ok(StoredRef {
        vault,
        path,
        selector,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canon's four shapes, byte-for-byte — a pinned exemplar of the
    /// shipped `md encode` grammar.
    #[test]
    fn encode_reproduces_the_shipped_c24_canon_byte_for_byte() {
        assert_eq!(
            encode("field-notes", "domains/media/logo.md", None).expect("page grain"),
            "obsidian://open?vault=field-notes&file=domains/media/logo.md",
        );
        assert_eq!(
            encode("field-notes", "domains/media/logo.md", Some("Design")).expect("heading"),
            "obsidian://advanced-uri?vault=field-notes&filepath=domains/media/logo.md&heading=Design",
        );
        assert_eq!(
            encode(
                "field-notes",
                "domains/media/logo.md",
                Some("^leaders-guideline")
            )
            .expect("block"),
            "obsidian://advanced-uri?vault=field-notes&filepath=domains/media/logo.md\
             &block=leaders-guideline",
        );
        // Space and non-ASCII encode, `/` stays literal.
        assert_eq!(
            encode("home wiki", "a b/ünï code.md", Some("A Heading")).expect("encoded"),
            "obsidian://advanced-uri?vault=home%20wiki&filepath=a%20b/%C3%BCn%C3%AF%20code.md\
             &heading=A%20Heading",
        );
    }

    /// Every byte the canon escapes, and every byte it leaves alone.
    #[test]
    fn the_percent_encoding_set_is_the_measured_one() {
        for (raw, encoded) in [
            ("a&b.md", "a%26b.md"),
            ("a?b.md", "a%3Fb.md"),
            ("a=b.md", "a%3Db.md"),
            ("a+b.md", "a%2Bb.md"),
            ("a#b.md", "a%23b.md"),
            ("a%b.md", "a%25b.md"),
            ("a b.md", "a%20b.md"),
        ] {
            assert_eq!(encode_component(raw), encoded, "{raw} must escape");
        }
        for literal in [
            "a'b.md",
            "a(b).md",
            "a~b_c-d.e.md",
            "a[b].md",
            "a,b;b:c.md",
            "a@b!.md",
            "a$b*.md",
            "deep/nested/path.md",
        ] {
            assert_eq!(
                encode_component(literal),
                literal,
                "{literal} must stay literal — `/` included",
            );
        }
    }

    /// Round trip, both directions, byte-identically.
    #[test]
    fn every_stored_form_decodes_back_to_the_parts_that_minted_it() {
        for (vault, path, selector) in [
            ("field-notes", "notes.md", None),
            ("field-notes", "24-01-retro/notes.md", Some("Design")),
            ("field-notes-sessions", "a/b.md", Some("^claim-1")),
            ("home wiki", "a b/ünï code.md", Some("A Heading")),
            ("v", "a&b=c?d#e+f%g.md", Some("h&i")),
            ("v", "dir/a:b.md", Some("Design/Sub")),
        ] {
            let uri = encode(vault, path, selector).expect("mints");
            let back = decode(&uri).expect("decodes");
            assert_eq!(back.vault, vault, "{uri}");
            assert_eq!(back.path, path, "{uri}");
            assert_eq!(back.selector.as_deref(), selector, "{uri}");
            // And the re-mint is byte-identical to the first mint.
            assert_eq!(
                encode(&back.vault, &back.path, back.selector.as_deref()).expect("re-mints"),
                uri,
                "the stored form must be a fixed point",
            );
        }
    }

    /// A hand-edited URI fails loudly — every refusal class names what it
    /// refused.
    #[test]
    fn a_hand_edited_stored_uri_refuses_and_names_the_canonical_spelling() {
        assert_eq!(
            decode("https://example.com"),
            Err(StoredError::NotStoredForm)
        );

        // `adv-uri` is not a canonical action.
        assert_eq!(
            decode("obsidian://adv-uri?vault=v&filepath=a.md&block=x"),
            Err(StoredError::UnknownAction {
                found: "adv-uri".to_string()
            }),
        );

        // `%2F` and `/` are the same address, different bytes — only the canon
        // check sees it.
        let err = decode("obsidian://open?vault=v&file=a%2Fb.md").expect_err("non-canonical");
        assert_eq!(
            err,
            StoredError::NonCanonicalEncoding {
                param: "file".to_string(),
                found: "a%2Fb.md".to_string(),
                canonical: "a/b.md".to_string(),
            },
        );
        assert!(
            err.to_string().contains("a/b.md"),
            "the refusal must name the canonical spelling: {err}",
        );

        // A dangling escape is never guessed past.
        assert!(matches!(
            decode("obsidian://open?vault=v&file=a%2zb.md"),
            Err(StoredError::NonCanonicalEncoding { .. }),
        ));
        // Reordered, extra and missing parameters.
        for bad in [
            "obsidian://open?file=a.md&vault=v",
            "obsidian://open?vault=v&file=a.md&extra=1",
            "obsidian://open?vault=v",
            "obsidian://advanced-uri?vault=v&filepath=a.md",
            "obsidian://advanced-uri?vault=v&file=a.md&heading=H",
        ] {
            assert!(
                matches!(decode(bad), Err(StoredError::BadParams { .. })),
                "{bad} must refuse as a parameter-shape failure",
            );
        }
        // Empty values and unaddressable paths.
        assert!(matches!(
            decode("obsidian://open?vault=&file=a.md"),
            Err(StoredError::EmptyValue { .. }),
        ));
        assert!(matches!(
            decode("obsidian://open?vault=v&file=../escape.md"),
            Err(StoredError::BadPath { .. }),
        ));
        assert!(
            matches!(
                decode("obsidian://open?vault=v&file=sessions:notes.md"),
                Err(StoredError::BadPath { .. }),
            ),
            "an ADDRESS is never a stored path — the head-colon arm of `confined`",
        );
    }

    /// The mint refuses what has no stored spelling and accepts the ordinary
    /// corpus.
    #[test]
    fn the_mint_refuses_only_what_has_no_stored_spelling() {
        assert!(encode("v", "notes.md", None).is_ok());
        assert!(encode("v", "a/b/c.md", Some("H")).is_ok());
        assert_eq!(
            encode("", "a.md", None),
            Err(StoredError::EmptyValue {
                param: "vault".to_string()
            }),
        );
        assert_eq!(
            encode("v", "a.md", Some("")),
            Err(StoredError::EmptyValue {
                param: "selector".to_string()
            }),
            "an empty selector is refused, never folded into page grain — folding \
             would lose the `#` and break the round trip silently",
        );
        assert_eq!(
            encode("v", "a.md", Some("^")),
            Err(StoredError::EmptyValue {
                param: "selector".to_string()
            }),
        );
        for bad in ["", "/abs.md", "../up.md", "sessions:notes.md"] {
            assert!(
                matches!(encode("v", bad, None), Err(StoredError::BadPath { .. })),
                "{bad} is not a corpus path",
            );
        }
    }
}
