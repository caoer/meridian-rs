//! The built-in [`RealisedGate`]: read the realise engine's pending-agent board
//! cards (its recorded terminal state) as a PURE observation.
//!
//! The realise engine (U3.5a) mints one governed board card per unrealised claim
//! through the U2.6 guarded create — `state: <terminal>`, `claim: <selector>`.
//! The card IS the recorded realisation state; attest reads it, never re-runs the
//! convergence loop (attest is a read gate). Scope law (d2 §2.5): a card counts
//! only when its claim selector's PAGE is the attest target — the pinner's own
//! compose closure. An upstream page's card is that page's refusal, not this one.

use std::path::Path;

use crate::{GateError, GateOutcome, RealisedGate, UNMET_STATES, UnmetClaim};

/// A realised-gate that reads pending-agent board cards from a workspace-relative
/// board directory. A card in an [`UNMET_STATES`] state whose claim page is the
/// target fails the gate; no such card (or no board dir) passes it.
#[derive(Debug, Clone)]
pub struct BoardGate {
    /// The workspace-relative directory the realise engine mints cards in (one
    /// file per claim selector).
    pub board_dir: String,
}

impl BoardGate {
    /// A board gate over `board_dir`.
    #[must_use]
    pub fn new(board_dir: impl Into<String>) -> Self {
        BoardGate {
            board_dir: board_dir.into(),
        }
    }
}

impl RealisedGate for BoardGate {
    fn evaluate(&self, root: &fs::WorkspaceRoot, target: &str) -> Result<GateOutcome, GateError> {
        let dir = root.0.join(self.board_dir.trim_end_matches('/'));
        // Deterministic order — the first unmet card in the target's own closure
        // wins (a stable, reproducible refusal).
        let mut names = match std::fs::read_dir(&dir) {
            Ok(rd) => {
                let mut ns = Vec::new();
                for entry in rd {
                    let entry = entry.map_err(|e| GateError {
                        reason: format!("read board dir '{}': {e}", self.board_dir),
                    })?;
                    let name = entry.file_name();
                    if Path::new(&name)
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
                    {
                        ns.push(name.to_string_lossy().into_owned());
                    }
                }
                ns
            }
            // No board directory ⇒ no recorded pending claims ⇒ the gate passes.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(GateOutcome::Realised),
            Err(e) => {
                return Err(GateError {
                    reason: format!("read board dir '{}': {e}", self.board_dir),
                });
            }
        };
        names.sort();

        for name in names {
            let rel = format!("{}/{name}", self.board_dir.trim_end_matches('/'));
            // A card that will not load is not the target's unrealised claim;
            // skip it (an unparseable board file is a board concern, not this
            // attest's — the scope law keeps attest to its own closure).
            let Ok(doc) = fs::load(root, Path::new(&rel)) else {
                continue;
            };
            let (Some(claim), Some(state)) = (fm_get(&doc, "claim"), fm_get(&doc, "state")) else {
                continue;
            };
            // Scope: the card's claim belongs to THIS target iff its selector page
            // (the part before the first `#`) is the target.
            let claim_page = claim.split_once('#').map_or(claim.as_str(), |(p, _)| p);
            if claim_page != target {
                continue;
            }
            if UNMET_STATES.contains(&state.as_str()) {
                return Ok(GateOutcome::Unmet(UnmetClaim {
                    claim,
                    state,
                    last_receipt: fm_get(&doc, "last_receipt"),
                }));
            }
        }
        Ok(GateOutcome::Realised)
    }
}

/// Read one frontmatter field off a parsed board card, using only the public
/// model surface (the frontmatter node's [`model::YamlMap`]) — the same read the
/// realise engine's `FieldEquals` check uses.
fn fm_get(doc: &model::Document, field: &str) -> Option<String> {
    fn find(node: &model::Node) -> Option<&model::YamlMap> {
        if let model::NodeKind::Frontmatter { map } = &node.kind {
            return Some(map);
        }
        node.children.iter().find_map(find)
    }
    find(&doc.root)
        .and_then(|m| m.0.iter().find(|(k, _)| k == field))
        .map(|(_, v)| v.clone())
}
