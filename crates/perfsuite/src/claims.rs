//! The claims registry: every perf claim is data in `claims.toml`; benches are
//! its evidence. A verdict joins a claim to a measurement:
//! PASS/FAIL (threshold held/broken), MEASURED (number, no gate yet),
//! UNTESTED (no measurement — the perf mirror of a `todo!()` body).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub id: String,
    pub title: String,
    /// Unit of `baseline`/`threshold`/measured value (e.g. `MB/s`, `ms`, `us`).
    pub metric: String,
    pub direction: Direction,
    /// Reference number to beat/hold (informational; the gate is `threshold`).
    #[serde(default)]
    pub baseline: Option<f64>,
    #[serde(default)]
    pub baseline_provenance: Option<String>,
    /// The gate. `direction=higher` fails below it, `lower` fails above it.
    #[serde(default)]
    pub threshold: Option<f64>,
    #[serde(default)]
    pub threshold_source: ThresholdSource,
    /// Which bench target produces the measurement.
    pub harness: String,
    #[serde(default)]
    pub recipe: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Higher,
    Lower,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThresholdSource {
    /// `threshold` in this file.
    #[default]
    Literal,
    /// Threshold owned by product data: `policy` ruleset `Budget{class, p99_us}`
    /// per assertion. Until rung 6 wires `policy::vocab()`, these render as
    /// MEASURED/UNTESTED, never PASS/FAIL.
    Ruleset,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    pub claim_id: String,
    pub measured: Option<f64>,
    pub verdict: VerdictKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum VerdictKind {
    Pass,
    Fail,
    Measured,
    Untested,
}

#[derive(Debug, Deserialize)]
struct ClaimsFile {
    claim: Vec<Claim>,
}

/// Load and sanity-check `claims.toml`.
pub fn load(path: &Path) -> Result<Vec<Claim>, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let file: ClaimsFile =
        toml::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
    let mut seen = std::collections::BTreeSet::new();
    for claim in &file.claim {
        if !seen.insert(&claim.id) {
            return Err(format!("duplicate claim id '{}'", claim.id));
        }
    }
    Ok(file.claim)
}

/// Bundled `claims.toml` next to this crate.
pub fn load_bundled() -> Result<Vec<Claim>, String> {
    load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("claims.toml"))
}

/// Join claims to measured headline values (keyed by claim id).
#[must_use]
pub fn join(claims: &[Claim], measured: &BTreeMap<String, f64>) -> Vec<Verdict> {
    claims
        .iter()
        .map(|claim| {
            let value = measured.get(&claim.id).copied();
            let verdict = match (value, claim.effective_threshold()) {
                (None, _) => VerdictKind::Untested,
                (Some(_), None) => VerdictKind::Measured,
                (Some(v), Some(t)) => {
                    let holds = match claim.direction {
                        Direction::Higher => v >= t,
                        Direction::Lower => v <= t,
                    };
                    if holds {
                        VerdictKind::Pass
                    } else {
                        VerdictKind::Fail
                    }
                }
            };
            Verdict {
                claim_id: claim.id.clone(),
                measured: value,
                verdict,
            }
        })
        .collect()
}

impl Claim {
    /// The gate actually in force. Ruleset-sourced thresholds are not
    /// resolvable until rung 6 (`policy::vocab()` is `todo!()`), so they gate
    /// nothing yet — by design, thresholds stay with their owner.
    #[must_use]
    pub fn effective_threshold(&self) -> Option<f64> {
        match self.threshold_source {
            ThresholdSource::Literal => self.threshold,
            ThresholdSource::Ruleset => None,
        }
    }
}
