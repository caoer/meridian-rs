//! The binding law (U4.3) — the door law that keeps the attested artifact and the
//! pages it arms ONE thing.
//!
//! # What this is (rulings § binding law; decision #6; security F2)
//! The attested artifact ([`crate::armed::ARMED_RULES_PATH`]) and each armed rule
//! PAGE are a bound pair: the artifact pins every armed rule at the rev a reviewer
//! approved. Editing ONE side without the other breaks the binding — a row edited
//! by hand (artifact side) or an armed law edited in place (page side). Both are
//! refused at the door as a **teaching refusal** ([`DoorLaw::BindingBreak`],
//! taxonomy row 9): the proper path is the ONE-act arming attestation.
//!
//! Deleting or renaming the artifact or the once-armed marker is a stronger break
//! — it attacks the enforcement substrate itself (deleting the marker is the
//! silent-disarm attack the fail-closed design defeats, security F2). That is the
//! **integrity floor** ([`DoorLaw::IndexIntegrity`], taxonomy row 10): a
//! structural refusal that `--force` does NOT escape.
//!
//! # `--force` is the sanctioned bypass (decision #6)
//! A binding break is force-escapable: a `--force` write lands, and the skip is
//! rendered as a `forced:` verdict (the caller loudly owns the bypass). The integrity floor
//! is not force-escapable — destroying the artifact or the marker is not a "check
//! to skip".
//!
//! # Purity
//! [`classify_door_law`] is a pure function of the write's op + target path + the
//! armed-page set. The reserved-path spellings mirror `fs::domain`; a cross-crate
//! test in `wire-serve` fails if the two ever drift.

use crate::change::ChangeOp;

/// The once-armed marker (mirrors `fs::domain::ATTESTED_MARKER_PATH`). Its
/// presence records that a workspace has ever been armed; the floor refuses its
/// deletion/rename.
pub const ATTESTED_MARKER_PATH: &str = "meridian/attested";

/// The pages the integrity floor protects: the enforcement substrate itself.
///
/// **Deleting the substrate must never read as disarming.** The marker fail-closes
/// the door on a once-armed workspace, so an artifact deletable by an ordinary
/// write would be the silent-disarm attack wearing the artifact's name.
const PROTECTED_SUBSTRATE: &[(&str, &str)] = &[
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

/// Which side of the artifact↔page binding a write broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingSide {
    /// The artifact side: a direct edit of the attested armed-rules artifact.
    Index,
    /// The page side: a direct edit/removal of an armed rule page.
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

/// The door-law verdict for one write, evaluated BEFORE rule evaluation on an
/// armed workspace. `Clear` lets the write flow to the rules; the other two are
/// refusals (one force-escapable, one structural).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoorLaw {
    /// No binding/integrity floor applies — evaluate the armed rules.
    Clear,
    /// A one-sided artifact↔page change (taxonomy row 9). Force-escapable: a
    /// `--force` write lands and the skip is rendered as a `forced:` verdict.
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
    /// Deletion or rename of the artifact or the once-armed marker (taxonomy row
    /// 10). A structural floor — `--force` does NOT escape it (security F2).
    IndexIntegrity {
        /// The protected file the write tried to remove.
        target: String,
        /// The teaching message citing the floor.
        teaching: String,
    },
}

/// Classify a write against the binding law + integrity floor. `op` is the write
/// op, `path` the target file path, `is_armed_page` answers whether a
/// workspace-relative page is armed in this workspace AT THIS WRITE'S PATH.
/// Called only on an armed workspace (a never-armed workspace is a bit-for-bit
/// no-op — this never runs).
///
/// Order (most-structural first):
/// 1. `remove` of the artifact or the marker → [`DoorLaw::IndexIntegrity`].
/// 2. `splice`/`create` of the artifact → [`DoorLaw::BindingBreak`] (index side).
/// 3. `splice`/`create`/`remove` of an armed rule PAGE → [`DoorLaw::BindingBreak`]
///    (file side).
/// 4. otherwise → [`DoorLaw::Clear`].
///
/// # Why the page side asks a set rather than a shape
/// The folder generation could answer "is this an armed law?" from the path alone
/// — `conventions/<slug>/CHECK.md` was the shape, and the slug was the key. A rule
/// page has no reserved shape: it is an ordinary page that registers by tag and
/// lives wherever its author put it. So the question is membership in what the
/// workspace ATTESTED, which is the artifact's rows, and it is answered by the
/// caller who already resolved them.
#[must_use]
pub fn classify_door_law(
    op: ChangeOp,
    path: &str,
    is_armed_page: &dyn Fn(&str) -> bool,
) -> DoorLaw {
    let segs = normalize_segments(path);
    let joined = segs.join("/");

    // 1. The integrity floor — the structural guard on the enforcement substrate.
    //    A remove (deletion; the remove-half of a rename) of the artifact or the
    //    marker is refused, never force-escaped.
    if op == ChangeOp::Remove
        && let Some(which) = protected_substrate(&joined)
    {
        return DoorLaw::IndexIntegrity {
            teaching: format!(
                "refused: the {which} `{joined}` may not be deleted or renamed at the door \
                 (integrity floor) — the once-armed marker fail-closes the door against silent \
                 disarm (security F2); disarm through the attest/arm path"
            ),
            target: joined,
        };
    }

    // 2. Binding break, artifact side — a direct write to the attested artifact.
    //    Arming/disarming is an attestation, never an ordinary door write.
    if joined == crate::armed::ARMED_RULES_PATH && matches!(op, ChangeOp::Splice | ChangeOp::Create)
    {
        return DoorLaw::BindingBreak {
            side: BindingSide::Index,
            teaching: format!(
                "refused: the attested armed-rules artifact `{joined}` is engine-managed — a \
                 direct edit of a row breaks the artifact↔page binding; arm or disarm through \
                 the attest/arm path, or `--force` to override (the skip is rendered as a \
                 `forced:` verdict naming the bypassed rule)"
            ),
            path: joined,
            legal_path: "the attest/arm path (the ONE-act attestation)".to_string(),
        };
    }

    // 3. Binding break, page side — a direct edit/removal of an ARMED rule page
    //    (it drifts the pinned rev under the artifact that still arms it).
    if is_armed_page(&joined) {
        return DoorLaw::BindingBreak {
            side: BindingSide::File,
            teaching: format!(
                "refused: `{joined}` is an armed rule page — a direct edit of its attested bytes \
                 breaks the artifact↔page binding (it drifts the pinned rev); re-arm at the new \
                 rev, or `--force` to override (the skip is rendered as a \
                 `forced:` verdict naming the bypassed rule)"
            ),
            path: joined.clone(),
            legal_path: format!("re-arm `{joined}` at the new evidence rev"),
        };
    }

    DoorLaw::Clear
}

/// A path's normalized workspace-relative segments: `/`-split, dropping empty /
/// `.` / `..` components, so a non-canonical spelling
/// (`./meridian/armed-rules.md`) cannot dodge the binding law. Mirrors
/// `fs::domain`'s reserved-path normalizer.
fn normalize_segments(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::armed::ARMED_RULES_PATH;

    /// An armed-page predicate over a fixed set of workspace-relative pages.
    fn armed<'a>(pages: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |p: &str| pages.contains(&p)
    }

    #[test]
    fn a_direct_edit_of_the_armed_rules_artifact_is_a_binding_break() {
        let is_armed = armed(&[]);
        let d = classify_door_law(ChangeOp::Splice, ARMED_RULES_PATH, &is_armed);
        let DoorLaw::BindingBreak { side, path, .. } = d else {
            panic!("a direct artifact edit must break the binding: {d:?}");
        };
        assert_eq!(side, BindingSide::Index);
        assert_eq!(path, ARMED_RULES_PATH);
    }

    #[test]
    fn non_canonical_artifact_spelling_still_caught() {
        let is_armed = armed(&[]);
        let dotted = format!("./{ARMED_RULES_PATH}");
        assert!(
            matches!(
                classify_door_law(ChangeOp::Splice, &dotted, &is_armed),
                DoorLaw::BindingBreak {
                    side: BindingSide::Index,
                    ..
                }
            ),
            "a `./`-prefixed artifact path must not dodge the binding law"
        );
    }

    /// The page-side break, re-keyed: an armed page is named by the artifact's
    /// rows, not by a reserved filename. A page ANYWHERE is bound once armed.
    #[test]
    fn direct_edit_of_an_armed_rule_page_is_a_page_side_break() {
        let is_armed = armed(&["team/rules/reviewer-not-owner.md"]);
        let d = classify_door_law(
            ChangeOp::Splice,
            "team/rules/reviewer-not-owner.md",
            &is_armed,
        );
        let DoorLaw::BindingBreak {
            side,
            path,
            teaching,
            ..
        } = d
        else {
            panic!("editing an armed rule page must break the binding: {d:?}");
        };
        assert_eq!(side, BindingSide::File);
        assert_eq!(path, "team/rules/reviewer-not-owner.md");
        assert!(
            teaching.contains("drifts the pinned rev"),
            "the teaching names why: {teaching}"
        );
    }

    /// Removing an armed page is the page-side break too — it is one-sided in the
    /// most complete way, and the artifact still arms what is gone.
    #[test]
    fn removing_an_armed_rule_page_is_a_page_side_break() {
        let is_armed = armed(&["rules/notify.md"]);
        assert!(
            matches!(
                classify_door_law(ChangeOp::Remove, "rules/notify.md", &is_armed),
                DoorLaw::BindingBreak {
                    side: BindingSide::File,
                    ..
                }
            ),
            "removing an armed page leaves the artifact arming a page that is gone"
        );
    }

    #[test]
    fn editing_an_unarmed_rule_page_is_clear() {
        // The page exists and may even register — but it is NOT armed, so editing
        // it freely is fine (edit, then arm). Only ARMED pages are bound.
        let is_armed = armed(&["rules/other.md"]);
        assert_eq!(
            classify_door_law(ChangeOp::Splice, "rules/draft.md", &is_armed),
            DoorLaw::Clear
        );
    }

    /// A non-canonical spelling of an armed page does not dodge the page side
    /// either — the predicate is asked the NORMALIZED path.
    #[test]
    fn non_canonical_armed_page_spelling_still_caught() {
        let is_armed = armed(&["rules/notify.md"]);
        assert!(
            matches!(
                classify_door_law(ChangeOp::Splice, "./rules/notify.md", &is_armed),
                DoorLaw::BindingBreak {
                    side: BindingSide::File,
                    ..
                }
            ),
            "the armed-page question is asked of the normalized path"
        );
    }

    /// **The armed-rules artifact is enforcement substrate, and deleting substrate
    /// must never read as disarming.** `--force` does not escape it.
    #[test]
    fn armed_rules_artifact_removal_is_index_integrity() {
        let is_armed = armed(&[]);
        let d = classify_door_law(ChangeOp::Remove, ARMED_RULES_PATH, &is_armed);
        let DoorLaw::IndexIntegrity { target, teaching } = d else {
            panic!("removing the armed-rules artifact hits the integrity floor: {d:?}");
        };
        assert_eq!(target, ARMED_RULES_PATH);
        assert!(
            teaching.contains("armed-rules") && teaching.contains("silent disarm"),
            "the refusal names the page and the attack: {teaching}"
        );

        // A non-canonical spelling does not dodge the floor.
        let dotted = format!("./{ARMED_RULES_PATH}");
        assert!(
            matches!(
                classify_door_law(ChangeOp::Remove, &dotted, &is_armed),
                DoorLaw::IndexIntegrity { .. }
            ),
            "a `./` spelling is the same page"
        );
    }

    #[test]
    fn marker_removal_is_index_integrity() {
        let is_armed = armed(&[]);
        assert!(
            matches!(
                classify_door_law(ChangeOp::Remove, ATTESTED_MARKER_PATH, &is_armed),
                DoorLaw::IndexIntegrity { .. }
            ),
            "removing the once-armed marker hits the integrity floor"
        );
    }

    /// The floor outranks the page-side break: an armed page that is ALSO the
    /// substrate would otherwise be force-escapable through the weaker arm.
    #[test]
    fn the_floor_outranks_the_page_side_break() {
        let is_armed = armed(&[ARMED_RULES_PATH, ATTESTED_MARKER_PATH]);
        assert!(matches!(
            classify_door_law(ChangeOp::Remove, ARMED_RULES_PATH, &is_armed),
            DoorLaw::IndexIntegrity { .. }
        ));
    }

    #[test]
    fn an_ordinary_content_write_is_clear() {
        let is_armed = armed(&["rules/reviewer.md"]);
        assert_eq!(
            classify_door_law(ChangeOp::Splice, "tasks/fix.md", &is_armed),
            DoorLaw::Clear
        );
        assert_eq!(
            classify_door_law(ChangeOp::Remove, "notes/plan.md", &is_armed),
            DoorLaw::Clear
        );
    }
}
