//! The binding law + `--truth` convergence (U4.3) — the door law that keeps the
//! attested INDEX and the convention files ONE thing, and the resolver that
//! deploys a divergence in an explicit direction.
//!
//! # What this is (rulings § binding law; decision #6; security F2)
//! The attested INDEX (`conventions/INDEX.md`) and each armed convention's
//! `CHECK.md` are a bound pair: the INDEX pins each armed convention at the
//! evidence rev a reviewer approved. Editing ONE side without the other breaks
//! the binding — a checkbox flipped by hand (index side) or an armed law edited
//! in place (file side). Both are refused at the door as a **teaching refusal**
//! ([`DoorLaw::BindingBreak`], taxonomy row 9): the proper path is the ONE-act
//! arming attestation, an explicit [`converge`] with `--truth`, or realise.
//!
//! Deleting or renaming the INDEX or the once-armed marker is a stronger break —
//! it attacks the enforcement substrate itself (deleting the marker is the
//! silent-disarm attack the fail-closed design defeats, security F2). That is
//! the **INDEX-integrity floor** ([`DoorLaw::IndexIntegrity`], taxonomy row 10):
//! a structural refusal that `--force` does NOT escape.
//!
//! # `--force` and `--truth` are the sanctioned bypass (decision #6)
//! A binding break is force-escapable: a `--force` write lands, and the skip is
//! journaled AND rendered (the caller loudly owns the bypass). The
//! INDEX-integrity floor is not force-escapable — destroying the INDEX/marker is
//! not a "check to skip". `--truth index|file` resolves a file↔index divergence
//! in an explicit direction ([`converge`]): `file` re-pins the INDEX at the live
//! evidence rev (deploy the edited law), `index` keeps the attested rev and
//! declares the file-side restore the convergence needs.
//!
//! # Purity
//! [`classify_door_law`] is a pure function of the write's op + target path +
//! the armed-slug set. [`converge`] injects a [`ConventionSource`] for the live
//! sweep (policy does no I/O, as `model` is). The reserved-path spellings mirror
//! `fs::domain`; a cross-crate test in `wire-serve` fails if the two ever drift.

use crate::change::ChangeOp;
use crate::check_eval::CheckLimits;
use crate::gate::ConventionSource;
use crate::index::{IndexEntry, arm, generate_index, parse_index_strict, sweep};

/// The attested INDEX page — the single page the door reads for the armed set
/// (mirrors `fs::domain::RESERVED_INDEX_PATH`).
pub const RESERVED_INDEX_PATH: &str = "conventions/INDEX.md";

/// The once-armed marker (mirrors `fs::domain::ATTESTED_MARKER_PATH`). Its
/// presence records that a workspace has ever been armed; the floor refuses its
/// deletion/rename.
pub const ATTESTED_MARKER_PATH: &str = "meridian/attested";

/// The pages the INDEX-integrity floor protects: the enforcement substrate
/// itself, whatever generation it belongs to.
///
/// The attested artifact ([`crate::armed::ARMED_RULES_PATH`]) is here for the same
/// reason the INDEX is, and the reason is not symmetry — it is that **deleting the
/// substrate must never read as disarming**. The marker fail-closes the door on a
/// once-armed workspace, so a floor that guarded only the older page would leave
/// the newer one deletable by an ordinary write while the marker still claimed the
/// workspace was armed.
const PROTECTED_SUBSTRATE: &[(&str, &str)] = &[
    (RESERVED_INDEX_PATH, "attested INDEX"),
    (
        crate::armed::ARMED_RULES_PATH,
        "attested armed-rules artifact",
    ),
    (ATTESTED_MARKER_PATH, "once-armed marker"),
];

/// The substrate page `joined` names, or `None` when it names none.
fn protected_substrate(joined: &str) -> Option<&'static str> {
    PROTECTED_SUBSTRATE
        .iter()
        .find(|(path, _)| *path == joined)
        .map(|(_, which)| *which)
}

/// The convention folder root, and the attested-law filename inside it — the
/// `conventions/<slug>/CHECK.md` shape whose rev the INDEX pins.
const CONVENTIONS_DIR: &str = "conventions";
const CHECK_FILE: &str = "CHECK.md";

/// Which side of the file↔index binding a write broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingSide {
    /// The index side: a direct edit of the attested INDEX (a checkbox flip).
    Index,
    /// The file side: a direct edit/removal of an armed convention's `CHECK.md`.
    File,
}

impl BindingSide {
    /// The wire-facing side word carried in a `binding_break{side}` refusal.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            BindingSide::Index => "index",
            BindingSide::File => "file",
        }
    }
}

/// The door-law verdict for one write, evaluated BEFORE convention evaluation on
/// an armed workspace. `Clear` lets the write flow to the conventions; the other
/// two are refusals (one force-escapable, one structural).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoorLaw {
    /// No binding/integrity floor applies — evaluate the armed conventions.
    Clear,
    /// A one-sided file↔index change (taxonomy row 9). Force-escapable: a
    /// `--force` write lands and the skip is journaled + rendered.
    BindingBreak {
        /// Which side the one-sided change touched.
        side: BindingSide,
        /// The engine-managed file the write targeted.
        path: String,
        /// The teaching message naming the break and the legal path.
        teaching: String,
        /// The legal path the refusal cites (the ONE-act proper path).
        legal_path: String,
    },
    /// Deletion or rename of the INDEX or the once-armed marker (taxonomy row
    /// 10). A structural floor — `--force` does NOT escape it (security F2).
    IndexIntegrity {
        /// The protected file (the INDEX or the marker) the write tried to remove.
        target: String,
        /// The teaching message citing the floor convention.
        teaching: String,
    },
}

/// Classify a write against the binding law + INDEX-integrity floor. `op` is the
/// write op, `path` the target file path, `is_armed_slug` answers whether a
/// convention slug is armed in this workspace. Called only on an armed
/// workspace (a never-armed workspace is a bit-for-bit no-op — this never runs).
///
/// Order (most-structural first):
/// 1. `remove` of the INDEX or the marker → [`DoorLaw::IndexIntegrity`].
/// 2. `splice`/`create` of the INDEX → [`DoorLaw::BindingBreak`] (index side).
/// 3. `splice`/`create`/`remove` of an armed convention's `CHECK.md` →
///    [`DoorLaw::BindingBreak`] (file side).
/// 4. otherwise → [`DoorLaw::Clear`].
#[must_use]
pub fn classify_door_law(
    op: ChangeOp,
    path: &str,
    is_armed_slug: &dyn Fn(&str) -> bool,
) -> DoorLaw {
    let segs = normalize_segments(path);
    let joined = segs.join("/");

    // 1. INDEX-integrity floor — the structural guard on the enforcement
    //    substrate. A remove (deletion; the remove-half of a rename) of the
    //    INDEX or the marker is refused, never force-escaped.
    if op == ChangeOp::Remove
        && let Some(which) = protected_substrate(&joined)
    {
        return DoorLaw::IndexIntegrity {
            teaching: format!(
                "refused: the {which} `{joined}` may not be deleted or renamed at the door \
                 (INDEX-integrity floor convention) — the once-armed marker fail-closes the \
                 door against silent disarm (security F2); disarm through the attest/arm path"
            ),
            target: joined,
        };
    }

    // 2. Binding break, index side — a direct write to the attested INDEX (a
    //    checkbox flip). Arming/disarming is an attestation, never an ordinary
    //    door write.
    if (joined == RESERVED_INDEX_PATH || joined == crate::armed::ARMED_RULES_PATH)
        && matches!(op, ChangeOp::Splice | ChangeOp::Create)
    {
        let which = protected_substrate(&joined).unwrap_or("attested INDEX");
        return DoorLaw::BindingBreak {
            side: BindingSide::Index,
            teaching: format!(
                "refused: the {which} `{joined}` is engine-managed — a direct edit (a \
                 checkbox flip) breaks the file↔index binding; arm or disarm through the \
                 attest/arm path, converge with `--truth index|file`, or `--force` to override \
                 (the skip is journaled and rendered)"
            ),
            path: joined,
            legal_path: "the attest/arm path (the ONE-act attestation)".to_string(),
        };
    }

    // 3. Binding break, file side — a direct edit/removal of an ARMED
    //    convention's attested CHECK.md (it drifts the pinned rev under the
    //    INDEX that still arms it).
    if segs.len() == 3
        && segs[0] == CONVENTIONS_DIR
        && segs[2] == CHECK_FILE
        && is_armed_slug(segs[1])
    {
        let slug = segs[1];
        return DoorLaw::BindingBreak {
            side: BindingSide::File,
            teaching: format!(
                "refused: convention `{slug}` is armed — a direct edit of its attested \
                 `{joined}` breaks the file↔index binding (it drifts the pinned rev); re-arm at \
                 the new rev, converge with `--truth index|file`, or `--force` to override (the \
                 skip is journaled and rendered)"
            ),
            path: joined,
            legal_path: format!("re-arm `{slug}` at the new evidence rev"),
        };
    }

    DoorLaw::Clear
}

/// A path's normalized workspace-relative segments: `/`-split, dropping empty /
/// `.` / `..` components, so a non-canonical spelling (`./conventions/INDEX.md`)
/// cannot dodge the binding law. Mirrors `fs::domain`'s reserved-path normalizer.
fn normalize_segments(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .collect()
}

// ── `--truth index|file` convergence ─────────────────────────────────────────

/// The explicit direction a file↔index divergence resolves in (`--truth`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Truth {
    /// The INDEX is the truth: keep the attested evidence rev; the live file
    /// must be restored to it (the declared file-side action).
    Index,
    /// The live file is the truth: re-pin the INDEX at the live evidence rev
    /// (deploy the edited law — realise-as-deploy).
    File,
}

/// One file-side action a convergence declares — the change the LIVE tree needs
/// that the pure resolver cannot itself make (it produces the INDEX side; the
/// file-side restore is realise-apply's / git's, a named boundary).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileAction {
    /// `--truth index`: the live `CHECK.md` drifted off the attested rev and
    /// must be restored to it (the INDEX stays; the file is reverted).
    RestoreFile {
        /// The armed convention whose file drifted.
        slug: String,
        /// The attested rev the file must be restored to.
        to_rev: String,
    },
    /// `--truth index`, dangling row: the armed convention's folder is gone and
    /// must be recreated at the attested rev (or the row disarmed).
    MissingFile {
        /// The armed convention whose folder vanished.
        slug: String,
        /// The attested rev the folder must be restored to.
        to_rev: String,
    },
}

/// The convergence of a file↔index divergence in one direction: the INDEX bytes
/// after, plus the file-side actions the live tree still needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Convergence {
    /// The attested INDEX page bytes after convergence.
    pub index_after: String,
    /// The live-tree actions the convergence declares (empty under `--truth
    /// file`, populated with restores under `--truth index`).
    pub file_actions: Vec<FileAction>,
}

/// Why a convergence could not be computed — the INDEX the caller handed in is
/// not a trustworthy attested page (same fail-closed reader the door uses).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvergeError {
    /// What is structurally wrong with the INDEX page.
    pub detail: String,
}

impl std::fmt::Display for ConvergeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cannot converge: {}", self.detail)
    }
}

impl std::error::Error for ConvergeError {}

/// Resolve a file↔index divergence in the stated direction. `current_index` is
/// the on-disk INDEX bytes; `source` reads the live convention folders (injected
/// — policy does no I/O); `truth` picks the direction.
///
/// - **[`Truth::File`]** — the live sweep is the truth. Every armed row is
///   re-swept and re-armed at its LIVE evidence rev (the drifted law is
///   deployed; the arming gate passes trivially since armed-rev == live rev).
///   A row whose folder no longer sweeps is DROPPED (dangling under file-truth).
///   No file action is needed — the files already ARE the truth.
/// - **[`Truth::Index`]** — the INDEX is the truth. `index_after` keeps the
///   attested revs verbatim; each drifted or dangling row emits a
///   [`FileAction`] naming the restore the live tree needs.
///
/// # Errors
/// [`ConvergeError`] when `current_index` is not a well-formed attested INDEX.
pub fn converge(
    current_index: &str,
    source: &dyn ConventionSource,
    limits: CheckLimits,
    truth: Truth,
) -> Result<Convergence, ConvergeError> {
    let declared =
        parse_index_strict(current_index).map_err(|c| ConvergeError { detail: c.detail })?;

    match truth {
        Truth::Index => {
            // The INDEX stands. Compare each declared armed row to its live rev
            // and declare the file-side restore where they diverge.
            let mut file_actions = Vec::new();
            for row in &declared {
                let files = source.files_for(&row.slug);
                match sweep(&*files, &row.slug, limits) {
                    Ok(swept) if swept.rev() == row.armed_rev => {} // converged already
                    Ok(_) => file_actions.push(FileAction::RestoreFile {
                        slug: row.slug.clone(),
                        to_rev: row.armed_rev.clone(),
                    }),
                    Err(_) => file_actions.push(FileAction::MissingFile {
                        slug: row.slug.clone(),
                        to_rev: row.armed_rev.clone(),
                    }),
                }
            }
            Ok(Convergence {
                index_after: current_index.to_string(),
                file_actions,
            })
        }
        Truth::File => {
            // The live sweep is the truth: re-pin every armed row at its live
            // rev; drop a row whose folder no longer sweeps (dangling).
            let mut rows: Vec<IndexEntry> = Vec::new();
            for row in &declared {
                let files = source.files_for(&row.slug);
                let Ok(swept) = sweep(&*files, &row.slug, limits) else {
                    continue; // dangling under file-truth — dropped
                };
                let live_rev = swept.rev().to_string();
                let armed = arm(swept, &live_rev, row.enforcement).map_err(|e| ConvergeError {
                    detail: format!("re-arming `{}` at its live rev failed: {e}", row.slug),
                })?;
                rows.push(armed);
            }
            Ok(Convergence {
                index_after: generate_index(&rows),
                file_actions: Vec::new(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convention::ConventionFiles;
    use crate::index::{Enforcement, armed_from_index};
    use std::collections::BTreeMap;

    // ── door-law classifier ──────────────────────────────────────────────────

    fn armed<'a>(slugs: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |s: &str| slugs.contains(&s)
    }

    #[test]
    fn checkbox_flip_of_the_index_is_a_binding_break_index_side() {
        let is_armed = armed(&["reviewer-not-owner"]);
        let d = classify_door_law(ChangeOp::Splice, RESERVED_INDEX_PATH, &is_armed);
        let DoorLaw::BindingBreak { side, path, .. } = d else {
            panic!("a direct INDEX splice must break the binding: {d:?}");
        };
        assert_eq!(side, BindingSide::Index);
        assert_eq!(path, RESERVED_INDEX_PATH);
    }

    #[test]
    fn non_canonical_index_spelling_still_caught() {
        let is_armed = armed(&[]);
        let d = classify_door_law(ChangeOp::Splice, "./conventions/INDEX.md", &is_armed);
        assert!(
            matches!(
                d,
                DoorLaw::BindingBreak {
                    side: BindingSide::Index,
                    ..
                }
            ),
            "a `./`-prefixed INDEX path must not dodge the binding law: {d:?}"
        );
    }

    #[test]
    fn direct_edit_of_an_armed_convention_check_is_a_file_side_break() {
        let is_armed = armed(&["reviewer-not-owner"]);
        let d = classify_door_law(
            ChangeOp::Splice,
            "conventions/reviewer-not-owner/CHECK.md",
            &is_armed,
        );
        assert!(
            matches!(
                d,
                DoorLaw::BindingBreak {
                    side: BindingSide::File,
                    ..
                }
            ),
            "editing an armed CHECK.md must break the binding (file side): {d:?}"
        );
    }

    #[test]
    fn editing_an_unarmed_convention_check_is_clear() {
        // The convention exists on disk but is NOT armed — editing it freely is
        // fine (edit, re-sweep, then arm). Only ARMED laws are bound.
        let is_armed = armed(&["other"]);
        let d = classify_door_law(
            ChangeOp::Splice,
            "conventions/draft-convention/CHECK.md",
            &is_armed,
        );
        assert_eq!(d, DoorLaw::Clear);
    }

    #[test]
    fn index_removal_is_index_integrity_not_binding_break() {
        let is_armed = armed(&[]);
        let d = classify_door_law(ChangeOp::Remove, RESERVED_INDEX_PATH, &is_armed);
        let DoorLaw::IndexIntegrity { target, .. } = d else {
            panic!("removing the INDEX hits the integrity floor: {d:?}");
        };
        assert_eq!(target, RESERVED_INDEX_PATH);
    }

    #[test]
    fn marker_removal_is_index_integrity() {
        let is_armed = armed(&[]);
        let d = classify_door_law(ChangeOp::Remove, ATTESTED_MARKER_PATH, &is_armed);
        assert!(
            matches!(d, DoorLaw::IndexIntegrity { .. }),
            "removing the once-armed marker hits the integrity floor: {d:?}"
        );
    }

    /// **The armed-rules artifact is enforcement substrate, and deleting substrate
    /// must never read as disarming.** The artifact succeeds the INDEX under tag
    /// registration, so it inherits the INDEX's structural floor rather than
    /// waiting to earn one: on a once-armed workspace the marker keeps the door
    /// fail-closed, so an artifact deletable by an ordinary write would be the
    /// silent-disarm attack wearing the new page's name. `--force` does not escape
    /// it, exactly as it does not escape the INDEX's.
    #[test]
    fn armed_rules_artifact_removal_is_index_integrity() {
        let is_armed = armed(&[]);
        let d = classify_door_law(ChangeOp::Remove, crate::armed::ARMED_RULES_PATH, &is_armed);
        let DoorLaw::IndexIntegrity { target, teaching } = d else {
            panic!("removing the armed-rules artifact hits the integrity floor: {d:?}");
        };
        assert_eq!(target, crate::armed::ARMED_RULES_PATH);
        assert!(
            teaching.contains("armed-rules") && teaching.contains("silent disarm"),
            "the refusal names the page and the attack: {teaching}"
        );

        // A non-canonical spelling does not dodge the floor.
        let dotted = format!("./{}", crate::armed::ARMED_RULES_PATH);
        assert!(
            matches!(
                classify_door_law(ChangeOp::Remove, &dotted, &is_armed),
                DoorLaw::IndexIntegrity { .. }
            ),
            "a `./` spelling is the same page"
        );
    }

    /// A direct EDIT of the artifact is the index-side binding break, not the
    /// floor: arming is an attestation, and hand-editing a row is the same act as
    /// flipping an INDEX checkbox. Force-escapable, and the skip is journaled.
    #[test]
    fn a_direct_edit_of_the_armed_rules_artifact_is_a_binding_break() {
        let is_armed = armed(&[]);
        let d = classify_door_law(ChangeOp::Splice, crate::armed::ARMED_RULES_PATH, &is_armed);
        let DoorLaw::BindingBreak { side, path, .. } = d else {
            panic!("a direct edit is the index-side binding break: {d:?}");
        };
        assert_eq!(side, BindingSide::Index);
        assert_eq!(path, crate::armed::ARMED_RULES_PATH);
    }

    #[test]
    fn an_ordinary_content_write_is_clear() {
        let is_armed = armed(&["reviewer-not-owner"]);
        assert_eq!(
            classify_door_law(ChangeOp::Splice, "tasks/fix.md", &is_armed),
            DoorLaw::Clear
        );
        assert_eq!(
            classify_door_law(ChangeOp::Remove, "notes/plan.md", &is_armed),
            DoorLaw::Clear
        );
    }

    // ── `--truth` convergence ────────────────────────────────────────────────

    #[derive(Clone)]
    struct MemConv(BTreeMap<String, String>);
    impl ConventionFiles for MemConv {
        fn read(&self, rel: &str) -> std::io::Result<String> {
            self.0.get(rel).cloned().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, format!("no {rel}"))
            })
        }
        fn exists(&self, rel: &str) -> bool {
            self.0.contains_key(rel)
        }
    }
    struct MemConventions(BTreeMap<String, MemConv>);
    impl ConventionSource for MemConventions {
        fn files_for<'a>(&'a self, slug: &str) -> Box<dyn ConventionFiles + 'a> {
            Box::new(
                self.0
                    .get(slug)
                    .cloned()
                    .unwrap_or_else(|| MemConv(BTreeMap::new())),
            )
        }
    }

    fn check_md(body_marker: &str) -> String {
        format!(
            "---\npaths:\n  - tasks/**\n---\n\n# reviewer-not-owner {body_marker}\n\n\
             ```starlark\ndef check_change(change):\n    pass\n```\n"
        )
    }

    fn conv_folder(check: &str) -> MemConv {
        let mut m = BTreeMap::new();
        m.insert(CHECK_FILE.to_string(), check.to_string());
        MemConv(m)
    }

    /// An INDEX arming `slug` at `level`, pinned to `check`'s live rev.
    fn armed_index(slug: &str, check: &str, level: Enforcement) -> String {
        let files = conv_folder(check);
        let swept = sweep(&files, slug, CheckLimits::default()).expect("sweeps");
        let rev = swept.rev().to_string();
        let armed = arm(swept, &rev, level).expect("arms at live rev");
        generate_index(&[armed])
    }

    /// The pinned rev the INDEX carries for `slug`.
    fn pinned_rev(index: &str, slug: &str) -> String {
        armed_from_index(index)
            .into_iter()
            .find(|r| r.slug == slug)
            .expect("slug is armed")
            .armed_rev
    }

    #[test]
    fn truth_file_re_pins_the_drifted_convention_at_its_live_rev() {
        // Arm at the ORIGINAL law, then drift the live CHECK.md.
        let original = check_md("v1");
        let index = armed_index("reviewer-not-owner", &original, Enforcement::Block);
        let attested = pinned_rev(&index, "reviewer-not-owner");

        let drifted = check_md("v2-edited");
        let live = MemConventions(BTreeMap::from([(
            "reviewer-not-owner".to_string(),
            conv_folder(&drifted),
        )]));

        let out = converge(&index, &live, CheckLimits::default(), Truth::File).expect("converges");
        let after = pinned_rev(&out.index_after, "reviewer-not-owner");
        assert_ne!(after, attested, "file-truth re-pins at the LIVE rev");
        assert_eq!(
            after,
            crate::page_rev(&drifted),
            "file-truth deploys the live law's rev"
        );
        assert!(
            out.file_actions.is_empty(),
            "file-truth needs no file action — the files ARE the truth"
        );
    }

    #[test]
    fn truth_index_keeps_the_attested_rev_and_declares_the_restore() {
        let original = check_md("v1");
        let index = armed_index("reviewer-not-owner", &original, Enforcement::Block);
        let attested = pinned_rev(&index, "reviewer-not-owner");

        let drifted = check_md("v2-edited");
        let live = MemConventions(BTreeMap::from([(
            "reviewer-not-owner".to_string(),
            conv_folder(&drifted),
        )]));

        let out = converge(&index, &live, CheckLimits::default(), Truth::Index).expect("converges");
        assert_eq!(
            pinned_rev(&out.index_after, "reviewer-not-owner"),
            attested,
            "index-truth keeps the attested rev"
        );
        assert_eq!(
            out.file_actions,
            vec![FileAction::RestoreFile {
                slug: "reviewer-not-owner".to_string(),
                to_rev: attested,
            }],
            "index-truth declares the file-side restore the divergence needs"
        );
    }

    #[test]
    fn both_directions_differ_on_the_same_divergence() {
        // The load-bearing symmetry: one divergence, two directions, two
        // observably different converged INDEX pins.
        let original = check_md("v1");
        let index = armed_index("reviewer-not-owner", &original, Enforcement::Block);
        let drifted = check_md("v2-edited");
        let live = MemConventions(BTreeMap::from([(
            "reviewer-not-owner".to_string(),
            conv_folder(&drifted),
        )]));

        let file = converge(&index, &live, CheckLimits::default(), Truth::File).unwrap();
        let idx = converge(&index, &live, CheckLimits::default(), Truth::Index).unwrap();
        assert_ne!(
            pinned_rev(&file.index_after, "reviewer-not-owner"),
            pinned_rev(&idx.index_after, "reviewer-not-owner"),
            "the two truth directions resolve the divergence differently"
        );
    }

    #[test]
    fn truth_file_drops_a_dangling_row() {
        // The INDEX arms a convention whose folder is gone (dangling).
        let index = armed_index("reviewer-not-owner", &check_md("v1"), Enforcement::Block);
        let empty = MemConventions(BTreeMap::new()); // no folders
        let out = converge(&index, &empty, CheckLimits::default(), Truth::File).unwrap();
        assert!(
            armed_from_index(&out.index_after).is_empty(),
            "file-truth drops the dangling row"
        );

        let idx = converge(&index, &empty, CheckLimits::default(), Truth::Index).unwrap();
        assert!(
            matches!(
                idx.file_actions.as_slice(),
                [FileAction::MissingFile { .. }]
            ),
            "index-truth declares the missing folder must be restored: {:?}",
            idx.file_actions
        );
    }
}
