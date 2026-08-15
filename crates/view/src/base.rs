//! `.base` (Obsidian Bases) content → projection rows
//! (`base-projection.md` §4).
//!
//! A leaf module of `view`, beside its only consumer. It parses one member's
//! bytes into the lifted columns and the carried JSON subtrees, and it
//! **never interprets the Bases language**: filter and formula expressions,
//! and the view `type` vocabulary, are Obsidian's — unversioned and evolving —
//! so they travel as verbatim text (§4.3).
//!
//! The parse is `serde_yaml` because Bases values nest arbitrarily; the
//! hand-rolled-scanner argument that admitted it into `config` for arbitrary
//! user frontmatter applies verbatim (§9). The `yaml_confinement` instrument's
//! permitted-taker set carries `view` with this paragraph as the stated
//! deviation.

use serde_yaml::Value as Yaml;

/// One member's parse — the shape [`crate::Rows`] stages.
///
/// An `error` member has EVERY content field `None` and zero children (§4.4);
/// the DDL CHECKs make the half-parsed state unrepresentable, and this type
/// makes it unconstructible.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Parsed {
    /// `None` = parsed as a YAML mapping; else the refusing parser's message.
    pub(crate) error: Option<String>,
    /// File-level `filters:` subtree as compact JSON (§4.2).
    pub(crate) filters: Option<String>,
    /// `properties:` subtree as compact JSON.
    pub(crate) properties: Option<String>,
    /// Every top-level key §4.5 does not lift, as one compact JSON object.
    pub(crate) extra: Option<String>,
    /// One entry per `views:` element, in document order.
    pub(crate) views: Vec<ParsedView>,
    /// One entry per `formulas:` key, in document order.
    pub(crate) formulas: Vec<ParsedFormula>,
}

/// One `views:` entry (§4.5: lifted where the shape holds, carried otherwise).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedView {
    pub(crate) name: Option<String>,
    pub(crate) type_: Option<String>,
    pub(crate) filters: Option<String>,
    /// Remaining view keys as one compact JSON object in written order, or the
    /// WHOLE entry when it is not a mapping.
    pub(crate) config: Option<String>,
}

/// One `formulas:` entry.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedFormula {
    pub(crate) name: String,
    /// The expression verbatim when scalar, else its compact JSON (§4.3).
    pub(crate) expr: String,
}

/// The alien row for a member whose bytes could not be read (§4.4) — the walk
/// SAW the path, so it comes back named, with the I/O error's own message.
pub(crate) fn unreadable(message: &str) -> Parsed {
    Parsed {
        error: Some(message.to_owned()),
        ..Parsed::default()
    }
}

/// Parse one member's bytes (§4).
///
/// Every refusal class of §4.4 lands as `error` with the refusing parser's own
/// message and no content: non-UTF-8 bytes, a shell script or markdown wearing
/// `.base`, YAML whose root is not a mapping, and **duplicate mapping keys
/// anywhere in the document** — the PINNED PARSER's rule, not YAML's:
/// `serde_yaml` refuses the whole document, so one duplicate `groupBy:` makes
/// the file an alien rather than last-key-wins. That verdict is deliberate:
/// tolerating duplicates would need a hand-rolled event-stream walk for a case
/// the live corpus has not produced.
pub(crate) fn parse(bytes: &[u8]) -> Parsed {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) => return unreadable(&e.to_string()),
    };
    let doc: Yaml = match serde_yaml::from_str(text) {
        Ok(doc) => doc,
        Err(e) => return unreadable(&e.to_string()),
    };
    let Yaml::Mapping(map) = doc else {
        return unreadable("expected a YAML mapping at the document root");
    };

    let mut parsed = Parsed::default();
    let mut extra = serde_yaml::Mapping::new();
    for (key, value) in map {
        match key.as_str() {
            Some("filters") => parsed.filters = Some(json_of(&value)),
            Some("properties") => parsed.properties = Some(json_of(&value)),
            // §4.5: a `views:`/`formulas:` that is not the modeled shape is
            // CARRIED intact in `extra` and makes no rows — the file is not an
            // alien, and a good `filters:` beside it still projects.
            Some("views") => match &value {
                Yaml::Sequence(entries) => {
                    parsed.views = entries.iter().map(parse_view).collect();
                }
                _ => drop(extra.insert(key, value)),
            },
            Some("formulas") => match &value {
                Yaml::Mapping(entries) => {
                    parsed.formulas = entries
                        .iter()
                        .filter_map(|(name, expr)| {
                            Some(ParsedFormula {
                                // A non-string formula key has no column to
                                // land in; it carries in `extra` instead of
                                // being renamed into one.
                                name: name.as_str()?.to_owned(),
                                expr: scalar_or_json(expr),
                            })
                        })
                        .collect();
                }
                _ => drop(extra.insert(key, value)),
            },
            _ => drop(extra.insert(key, value)),
        }
    }
    if !extra.is_empty() {
        parsed.extra = Some(json_of(&Yaml::Mapping(extra)));
    }
    parsed
}

/// One `views:` entry under the §4.5 lifting law: an entry that is not a
/// mapping lands whole in `config` with every lifted column NULL; `name`/`type`
/// present but not strings stay in `config` too.
fn parse_view(entry: &Yaml) -> ParsedView {
    let Yaml::Mapping(map) = entry else {
        return ParsedView {
            name: None,
            type_: None,
            filters: None,
            config: Some(json_of(entry)),
        };
    };
    let mut view = ParsedView {
        name: None,
        type_: None,
        filters: None,
        config: None,
    };
    let mut config = serde_yaml::Mapping::new();
    for (key, value) in map {
        match (key.as_str(), value.as_str()) {
            (Some("name"), Some(text)) => view.name = Some(text.to_owned()),
            // OPEN SET: no enum, no CHECK — when Obsidian grows a view type,
            // the projection carries it on day one (§4.3).
            (Some("type"), Some(text)) => view.type_ = Some(text.to_owned()),
            (Some("filters"), _) => view.filters = Some(json_of(value)),
            _ => drop(config.insert(key.clone(), value.clone())),
        }
    }
    if !config.is_empty() {
        view.config = Some(json_of(&Yaml::Mapping(config)));
    }
    view
}

/// A scalar's own text verbatim, or the compact JSON of a non-scalar (§4.5).
/// An expression string therefore survives byte-exact — `this.note["tag"]`
/// arrives as it was written, not as a re-quoted JSON string.
fn scalar_or_json(value: &Yaml) -> String {
    value.as_str().map_or_else(|| json_of(value), str::to_owned)
}

/// A YAML subtree as compact JSON, written order preserved, structure
/// preserving (§4.2): mappings → objects, sequences → arrays, scalars → JSON
/// scalars, expression strings → JSON strings, byte-for-byte. Nothing is
/// normalized, sorted, defaulted, or interpreted — two spellings of one query
/// stay two texts.
///
/// A non-string mapping key renders as its own JSON text (JSON has no other
/// key space), which is a rendering choice inside a carrier column, never a
/// lift.
fn json_of(value: &Yaml) -> String {
    json_value(value).to_string()
}

fn json_value(value: &Yaml) -> serde_json::Value {
    use serde_json::Value as Json;
    match value {
        Yaml::Null => Json::Null,
        Yaml::Bool(b) => Json::Bool(*b),
        Yaml::Number(n) => n
            .as_i64()
            .map(Json::from)
            .or_else(|| n.as_u64().map(Json::from))
            .or_else(|| {
                n.as_f64()
                    .and_then(serde_json::Number::from_f64)
                    .map(Json::Number)
            })
            // A YAML number JSON cannot hold (±inf, nan) keeps its own text
            // rather than becoming null — carried, not destroyed.
            .unwrap_or_else(|| Json::String(format!("{n:?}"))),
        Yaml::String(s) => Json::String(s.clone()),
        Yaml::Sequence(items) => Json::Array(items.iter().map(json_value).collect()),
        Yaml::Mapping(map) => Json::Object(
            map.iter()
                .map(|(k, v)| {
                    let key = k
                        .as_str()
                        .map_or_else(|| json_value(k).to_string(), str::to_owned);
                    (key, json_value(v))
                })
                .collect(),
        ),
        // A YAML tag wraps its value; the projection carries the value (the
        // tag is Obsidian language the engine does not interpret).
        Yaml::Tagged(tagged) => json_value(&tagged.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expression_strings_survive_byte_exact() {
        let parsed = parse(b"filters:\n  and:\n    - this.note[\"tag\"]\n");
        assert_eq!(
            parsed.filters.as_deref(),
            Some(r#"{"and":["this.note[\"tag\"]"]}"#)
        );
    }

    #[test]
    fn a_non_mapping_root_is_an_alien_with_no_content() {
        let parsed = parse(b"#!/bin/sh\necho hi\n");
        assert!(parsed.error.is_some(), "a shell script is an alien");
        assert_eq!(parsed.filters, None);
        assert!(parsed.views.is_empty() && parsed.formulas.is_empty());
    }

    #[test]
    fn duplicate_mapping_keys_refuse_the_whole_document() {
        let parsed = parse(b"views:\n  - type: table\n    groupBy: a\n    groupBy: b\n");
        assert!(
            parsed.error.is_some(),
            "the pinned parser refuses duplicates (§4.4), never last-key-wins"
        );
        assert!(parsed.views.is_empty(), "an error row has zero children");
    }

    #[test]
    fn an_unmodeled_views_shape_carries_in_extra_and_makes_no_rows() {
        let parsed = parse(b"filters: file.hasTag(\"x\")\nviews: nope\n");
        assert!(parsed.error.is_none(), "not an alien — §4.5 carries it");
        assert!(parsed.views.is_empty());
        assert_eq!(parsed.extra.as_deref(), Some(r#"{"views":"nope"}"#));
        assert_eq!(
            parsed.filters.as_deref(),
            Some(r#""file.hasTag(\"x\")""#),
            "a bare-expression filters: is a JSON string, not an error"
        );
    }

    #[test]
    fn a_non_mapping_view_entry_carries_whole_in_config() {
        let parsed = parse(b"views:\n  - just-a-string\n");
        assert_eq!(parsed.views.len(), 1);
        assert_eq!(parsed.views[0].name, None);
        assert_eq!(parsed.views[0].type_, None);
        assert_eq!(
            parsed.views[0].config.as_deref(),
            Some(r#""just-a-string""#)
        );
    }
}
