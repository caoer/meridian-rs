//! The attested INDEX generator (U1.4) — sweep `conventions/` into a living
//! checklist page, and the arming gate that pins a convention at an attested rev.
//!
//! # What this is (rulings § arming)
//! The INDEX is the ONE page the door reads to know which conventions are armed —
//! a living checklist swept from the `conventions/` folder grammar (U1.3). Each
//! convention is one row:
//!
//! ```text
//! - [x] **reviewer-not-owner** · block · `1aa7b939cee536b2` · [[conventions/reviewer-not-owner/CHECK.md]] · `tasks/**`
//!   └ checkbox  └ slug              └ severity └ pinned rev       └ evidence link                              └ scope
//! ```
//!
//! - **checkbox** — `[x]` armed (the door enforces it), `[ ]` off;
//! - **severity** — `off` / `warn` / `block`, the enforcement level;
//! - **pinned rev** — the evidence rev the row is pinned at (`blake3(CHECK.md)[:16]`);
//! - **evidence link** — a wikilink to the convention's `CHECK.md` (the attested law);
//! - **scope** — the convention's `paths:` globs.
//!
//! Rows sort by severity descending (block before warn before off), then slug
//! ascending — the most-enforced conventions read first.
//!
//! # O(armed) single-file read (rulings § arming)
//! The door never re-walks `conventions/` to learn the armed set — it reads THIS
//! ONE page and takes the `[x]` rows. [`armed_from_index`] is that read path:
//! one file, O(armed) rows out.
//!
//! # Arming requires `report-rev == armed-rev` (the drift gate)
//! Arming a convention is an attestation — a reviewer approves a convention AT the
//! evidence rev they read (the `armed-rev`). [`arm`] admits that attestation ONLY
//! when the convention's live rev (the freshly-swept `report-rev`) still equals the
//! approved `armed-rev`. If the evidence drifted since approval, arming is refused
//! ([`ArmError::Drift`]) — a law that changed out from under its approval never
//! silently arms. The armed [`IndexEntry`] is sealed to [`arm`]: the only way to a
//! `warn`/`block` row is through the gate.

use crate::check_eval::CheckLimits;
use crate::convention::{ConventionFiles, LoadError, load_convention};

/// The `CHECK.md` filename inside a convention folder — the attested law file whose
/// bytes the pinned rev hashes and whose path the evidence link points at.
const CHECK_FILE: &str = "CHECK.md";

/// How hard the door acts on a convention — the INDEX's `severity` column. `Off` is
/// unarmed (the door ignores the convention); `Warn` annotates but lands the write;
/// `Block` refuses the write. Declaration order IS the severity order (`Off` <
/// `Warn` < `Block`), so the default sort is `Ord` descending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Enforcement {
    /// Unarmed — the door ignores this convention (rendered `[ ]`).
    Off,
    /// Armed as advisory — the door annotates the write but lands it.
    Warn,
    /// Armed as a gate — the door refuses the write.
    Block,
}

impl Enforcement {
    /// The severity word the INDEX renders and [`armed_from_index`] parses.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Enforcement::Off => "off",
            Enforcement::Warn => "warn",
            Enforcement::Block => "block",
        }
    }

    /// Parse a severity word back from an INDEX row, or `None` if it is not one of
    /// `off` / `warn` / `block`.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "off" => Some(Enforcement::Off),
            "warn" => Some(Enforcement::Warn),
            "block" => Some(Enforcement::Block),
            _ => None,
        }
    }

    /// Whether this level arms the convention (the door enforces `warn`/`block`,
    /// ignores `off`) — the checkbox is `[x]` iff armed.
    #[must_use]
    pub fn is_armed(self) -> bool {
        self != Enforcement::Off
    }
}

/// One swept convention as an INDEX row: slug, scope, the evidence rev it is pinned
/// at, and its enforcement level. Construction is sealed to [`sweep`] (an unarmed
/// `Off` row) and [`arm`] (the drift-gated `warn`/`block` row) — a hand-built armed
/// row cannot bypass the `report-rev == armed-rev` gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    slug: String,
    scope: Vec<String>,
    rev: String,
    enforcement: Enforcement,
}

impl IndexEntry {
    /// The convention's subject slug.
    #[must_use]
    pub fn slug(&self) -> &str {
        &self.slug
    }

    /// The convention's `paths:` scope globs.
    #[must_use]
    pub fn scope(&self) -> &[String] {
        &self.scope
    }

    /// The evidence rev the row is pinned at (`blake3(CHECK.md)[:16]`). For an armed
    /// row this equals the approved `armed-rev` by the [`arm`] gate.
    #[must_use]
    pub fn rev(&self) -> &str {
        &self.rev
    }

    /// The row's enforcement level.
    #[must_use]
    pub fn enforcement(&self) -> Enforcement {
        self.enforcement
    }

    /// Render this row as one checklist line (no trailing newline).
    fn render(&self) -> String {
        let checkbox = if self.enforcement.is_armed() {
            "[x]"
        } else {
            "[ ]"
        };
        format!(
            "- {checkbox} **{slug}** · {sev} · `{rev}` · [[conventions/{slug}/{check}]] · `{scope}`",
            slug = self.slug,
            sev = self.enforcement.as_str(),
            rev = self.rev,
            check = CHECK_FILE,
            scope = self.scope.join(", "),
        )
    }
}

/// A convention read back from an INDEX `[x]` row — the door's O(armed) view: which
/// convention is armed, at what severity, pinned at which evidence rev. The read
/// counterpart of the sealed [`IndexEntry`]; the file IS the attestation, so this
/// carries no gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmedRef {
    /// The armed convention's slug.
    pub slug: String,
    /// The armed severity (`warn` or `block` — an `off` row is never armed).
    pub enforcement: Enforcement,
    /// The evidence rev the row is pinned at — the door drift-checks against it.
    pub armed_rev: String,
}

/// Why arming was refused. Arming's ONE gate is `report-rev == armed-rev`; a
/// mismatch is [`ArmError::Drift`] — the convention's evidence changed since the
/// reviewer approved it, so the approval no longer covers the live law.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArmError {
    /// The live (freshly-swept) evidence rev no longer equals the reviewer-approved
    /// rev — the convention drifted since approval, arming refused (fail-closed).
    Drift {
        /// The rev the reviewer approved (what they read).
        armed_rev: String,
        /// The rev the convention resolves to now (the sweep).
        report_rev: String,
    },
}

impl std::fmt::Display for ArmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArmError::Drift {
                armed_rev,
                report_rev,
            } => write!(
                f,
                "arming refused: evidence drifted — approved rev `{armed_rev}` but the \
                 convention now reads `{report_rev}` (arming requires report-rev == armed-rev)"
            ),
        }
    }
}

impl std::error::Error for ArmError {}

/// The evidence rev of a `CHECK.md`: `blake3(bytes)[:16]`, 16 lowercase hex — the
/// same rev law the world model mints (contract §1). Two byte-identical CHECK files
/// share a rev; any edit (predicate, scope, or prose) moves it, so the arming gate
/// catches every drift of the attested law.
#[must_use]
pub fn evidence_rev(check_md: &str) -> String {
    blake3::hash(check_md.as_bytes()).to_hex().as_str()[..16].to_string()
}

/// Sweep one convention folder into an unarmed (`Off`) INDEX row: validate it
/// through the loader (U1.3 folder grammar + capability ceiling), then pin its
/// evidence rev. Policy stays I/O-free — `files` is the caller-injected accessor;
/// the disk edge enumerates `conventions/` and calls this per slug.
///
/// # Errors
/// [`LoadError`] — the convention did not load (bad slug, missing/malformed
/// `CHECK.md`, a deferred capability, or a predicate that failed the load gate). A
/// convention that does not load is never a row.
pub fn sweep(
    files: &dyn ConventionFiles,
    slug: &str,
    limits: CheckLimits,
) -> Result<IndexEntry, LoadError> {
    let convention = load_convention(slug, files, limits)?;
    let check_md = files
        .read(CHECK_FILE)
        .map_err(|e| LoadError::CheckMissing {
            detail: e.to_string(),
        })?;
    Ok(IndexEntry {
        slug: convention.slug().to_string(),
        scope: convention.scope().to_vec(),
        rev: evidence_rev(&check_md),
        enforcement: Enforcement::Off,
    })
}

/// Arm a swept convention at `level`, pinned to the reviewer-approved `armed_rev` —
/// the attestation gate. Admits the arming ONLY when the swept row's live rev
/// (`report-rev`) equals `armed_rev`; a mismatch is refused [`ArmError::Drift`]
/// (fail-closed on evidence drift). `swept` is consumed — arming transforms the row.
///
/// Arming to [`Enforcement::Off`] is a deliberate attested-off (the reviewer read
/// the convention at this rev and chose not to enforce it); it passes the same
/// gate, so an attested-off row still pins the rev it was reviewed at.
///
/// # Errors
/// [`ArmError::Drift`] when `swept.rev() != armed_rev`.
pub fn arm(swept: IndexEntry, armed_rev: &str, level: Enforcement) -> Result<IndexEntry, ArmError> {
    if swept.rev != armed_rev {
        return Err(ArmError::Drift {
            armed_rev: armed_rev.to_string(),
            report_rev: swept.rev,
        });
    }
    Ok(IndexEntry {
        enforcement: level,
        ..swept
    })
}

/// The INDEX page title heading.
const INDEX_TITLE: &str = "# Attested conventions INDEX";

/// The INDEX page's fixed preamble (below the title), teaching the row grammar and
/// the arming law so the living page is self-describing.
const INDEX_PREAMBLE: &str = "\
Swept from `conventions/`. Each row is a convention: its enforcement severity \
(off/warn/block), the evidence rev it is pinned at, a link to its CHECK.md, and its \
path scope. A `[x]` row is armed — the door enforces it. Arming requires the live \
evidence rev to equal the pinned rev; a drifted convention is refused, never \
silently armed.";

/// Render the checklist rows for `entries`, sorted by severity descending then slug
/// ascending — one `- [ ]` / `- [x]` line each, `\n`-joined, no trailing newline.
/// This is the byte-exact row block the INDEX carries and the generator fixture
/// pins.
#[must_use]
pub fn render_rows(entries: &[IndexEntry]) -> String {
    let mut sorted: Vec<&IndexEntry> = entries.iter().collect();
    // Severity descending (block first), then slug ascending.
    sorted.sort_by(|a, b| {
        b.enforcement
            .cmp(&a.enforcement)
            .then_with(|| a.slug.cmp(&b.slug))
    });
    sorted
        .iter()
        .map(|e| e.render())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Generate the full living INDEX page: the title, the fixed preamble, and the
/// sorted checklist rows, terminated by a trailing newline. The disk edge writes
/// this to `conventions/INDEX.md`; policy stays I/O-free and returns the bytes.
#[must_use]
pub fn generate_index(entries: &[IndexEntry]) -> String {
    format!(
        "{title}\n\n{preamble}\n\n{rows}\n",
        title = INDEX_TITLE,
        preamble = INDEX_PREAMBLE,
        rows = render_rows(entries),
    )
}

/// Read the armed set back from an INDEX page — the door's O(armed) single-file
/// path. Scans for `[x]` rows and returns one [`ArmedRef`] each (slug, severity,
/// pinned rev); `[ ]` (off) rows are skipped. A line that does not match the row
/// grammar is ignored, so title/preamble prose never becomes a false armed entry.
#[must_use]
pub fn armed_from_index(index: &str) -> Vec<ArmedRef> {
    index.lines().filter_map(parse_armed_line).collect()
}

/// Parse one armed (`- [x] …`) checklist line into an [`ArmedRef`], or `None` when
/// the line is off, prose, or malformed.
fn parse_armed_line(line: &str) -> Option<ArmedRef> {
    let rest = line.strip_prefix("- [x] ")?;
    let mut fields = rest.split(" · ");
    let slug = fields
        .next()?
        .trim_start_matches("**")
        .trim_end_matches("**");
    let enforcement = Enforcement::parse(fields.next()?)?;
    let armed_rev = fields.next()?.trim_matches('`');
    if slug.is_empty() || armed_rev.is_empty() {
        return None;
    }
    Some(ArmedRef {
        slug: slug.to_string(),
        enforcement,
        armed_rev: armed_rev.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// An in-memory convention folder for the sweep fixtures.
    struct MemFiles(BTreeMap<String, String>);

    impl MemFiles {
        fn with(rel: &str, body: &str) -> Self {
            let mut m = BTreeMap::new();
            m.insert(rel.to_string(), body.to_string());
            Self(m)
        }
    }

    impl ConventionFiles for MemFiles {
        fn read(&self, rel_path: &str) -> std::io::Result<String> {
            self.0.get(rel_path).cloned().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, format!("no {rel_path}"))
            })
        }
        fn exists(&self, rel_path: &str) -> bool {
            self.0.contains_key(rel_path)
        }
    }

    fn check_md(scope: &str) -> String {
        format!(
            "---\npaths:\n  - {scope}\n---\n\n# c\n\n```starlark\ndef check_change(change):\n    pass\n```\n"
        )
    }

    fn sweep_conv(slug: &str, scope: &str) -> IndexEntry {
        let files = MemFiles::with(CHECK_FILE, &check_md(scope));
        sweep(&files, slug, CheckLimits::default()).expect("a well-formed convention sweeps")
    }

    #[test]
    fn severity_orders_block_over_warn_over_off() {
        assert!(Enforcement::Block > Enforcement::Warn);
        assert!(Enforcement::Warn > Enforcement::Off);
    }

    #[test]
    fn evidence_rev_is_blake3_16() {
        let rev = evidence_rev("hello");
        assert_eq!(rev.len(), 16, "16 hex chars");
        assert!(rev.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(
            evidence_rev("a"),
            evidence_rev("b"),
            "distinct bytes differ"
        );
    }

    #[test]
    fn sweep_pins_the_evidence_rev_and_starts_off() {
        let scope = "tasks/**";
        let entry = sweep_conv("reviewer-not-owner", scope);
        assert_eq!(entry.slug(), "reviewer-not-owner");
        assert_eq!(entry.scope(), &["tasks/**".to_string()]);
        assert_eq!(
            entry.enforcement(),
            Enforcement::Off,
            "a fresh sweep is off"
        );
        assert_eq!(
            entry.rev(),
            evidence_rev(&check_md(scope)),
            "pinned rev is blake3(CHECK.md)[:16]"
        );
    }

    #[test]
    fn arm_admits_when_report_rev_equals_armed_rev() {
        let entry = sweep_conv("c", "tasks/**");
        let approved = entry.rev().to_string();
        let armed = arm(entry, &approved, Enforcement::Block).expect("matching rev arms");
        assert_eq!(armed.enforcement(), Enforcement::Block);
        assert_eq!(armed.rev(), approved, "the armed row pins the approved rev");
    }

    /// Sweep a convention and arm it at `level`, pinned to its own live rev.
    fn armed_conv(slug: &str, scope: &str, level: Enforcement) -> IndexEntry {
        let swept = sweep_conv(slug, scope);
        let rev = swept.rev().to_string();
        arm(swept, &rev, level).expect("arming at the live rev succeeds")
    }

    #[test]
    fn rows_sort_severity_desc_then_slug_asc() {
        let a_off = sweep_conv("alpha", "a/**");
        let z_block = armed_conv("zeta", "z/**", Enforcement::Block);
        let m_warn = armed_conv("mid", "m/**", Enforcement::Warn);
        let rows = render_rows(&[a_off, z_block, m_warn]);
        let order: Vec<&str> = rows
            .lines()
            .map(|l| l.split("**").nth(1).unwrap())
            .collect();
        assert_eq!(
            order,
            vec!["zeta", "mid", "alpha"],
            "block, then warn, then off"
        );
    }

    #[test]
    fn generate_index_carries_title_preamble_and_rows() {
        let entry = sweep_conv("c", "tasks/**");
        let page = generate_index(&[entry]);
        assert!(page.starts_with(INDEX_TITLE), "titled");
        assert!(
            page.contains("Swept from `conventions/`"),
            "preamble present"
        );
        assert!(page.contains("- [ ] **c** · off · "), "the row renders");
        assert!(page.ends_with('\n'), "trailing newline");
    }

    #[test]
    fn armed_from_index_round_trips_only_armed_rows() {
        let off = sweep_conv("a-off", "a/**");
        let warn = armed_conv("b-warn", "b/**", Enforcement::Warn);
        let block_seed = sweep_conv("c-block", "c/**");
        let block_rev = block_seed.rev().to_string();
        let block = arm(block_seed, &block_rev, Enforcement::Block).unwrap();

        let page = generate_index(&[off, warn, block]);
        let armed = armed_from_index(&page);
        assert_eq!(armed.len(), 2, "only the two armed rows read back");
        // Block sorts first, then warn.
        assert_eq!(
            armed[0],
            ArmedRef {
                slug: "c-block".into(),
                enforcement: Enforcement::Block,
                armed_rev: block_rev,
            }
        );
        assert_eq!(armed[1].slug, "b-warn");
        assert_eq!(armed[1].enforcement, Enforcement::Warn);
    }

    #[test]
    fn armed_from_index_ignores_prose_lines() {
        // Title + preamble prose must never parse as an armed row.
        let page = generate_index(&[sweep_conv("c", "tasks/**")]);
        assert!(
            armed_from_index(&page).is_empty(),
            "an off-only page arms nothing"
        );
    }
}
