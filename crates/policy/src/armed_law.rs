//! A workspace's attested armed law at one path — and the surface reporting
//! every way it could not be honored ([`ArmedFault`]: absent, corrupt, zero
//! rows, red row, unloadable row, unevaluable law).
//!
//! [`resolve_armed_law`] pivots on the once-armed marker so a workspace that
//! has ever been armed fails CLOSED rather than reading as "nothing armed".
//! Output is the [`ArmedLaw`] the gate consumes at the write's own path.

use crate::armed::{ARMED_RULES_PATH, ArmedRow, Mode, PageSource, RedRow, Redness, parse_artifact};
use crate::check_eval::CheckLimits;
use crate::registration::{PageRef, RuleId, ScopeLayer, register_page};
use crate::rule::{Rule, RuleLoadError, load_rule};

/// One way a once-armed workspace's attested law could not be honored.
///
/// A closed vocabulary: every fault an operator can be told about is here, so a
/// host adds a channel rather than a word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArmedFault {
    /// The workspace has been armed, but its attested artifact cannot be read.
    /// Absent and unreadable are one fault: neither says what the law is.
    Missing {
        /// The artifact's workspace path.
        path: String,
    },
    /// The artifact is present and is not a trustworthy attested armed set. A
    /// corrupt page never reads as "nothing armed" — that would be a gate-disabling
    /// edit dressed as a parse.
    Corrupt {
        /// The artifact's workspace path.
        path: String,
        /// What is structurally wrong with it.
        detail: String,
    },
    /// The artifact parses and attests NO row at all on a once-armed workspace —
    /// the shape a row deletion leaves behind (module docs, "Zero rows").
    Disarmed {
        /// The artifact's workspace path.
        path: String,
    },
    /// A row governing this path reddened: its pinned page drifted, vanished, or
    /// no longer registers a kind whose vocabulary admits its mode.
    Red(RedRow),
    /// A row governing this path stands — its pinned bytes are the attested bytes —
    /// but those bytes do not LOAD as the rule they register as. Registration and
    /// declaration are two layers, so an armed page can still be undeclarable.
    Unloadable {
        /// The attested row.
        row: ArmedRow,
        /// The loader's own refusal.
        error: RuleLoadError,
    },
    /// A row loaded, but its law could not be EVALUATED over the change in hand —
    /// a budget, parse, or runtime fault in the predicate. The only fault a
    /// consumer can raise; the row it carries is [`resolve_armed_law`]'s, so it
    /// is not constructible from a bare id.
    Unevaluable {
        /// The attested row whose law faulted.
        row: ArmedRow,
        /// The evaluator's own message.
        detail: String,
    },
}

impl ArmedFault {
    /// Whether this fault must REFUSE the write rather than be reported and
    /// survived — the ruled kind split (module docs).
    #[must_use]
    pub fn refuses(&self) -> bool {
        match self {
            ArmedFault::Missing { .. }
            | ArmedFault::Corrupt { .. }
            | ArmedFault::Disarmed { .. } => true,
            ArmedFault::Red(red) => red.row().mode().enforces(),
            ArmedFault::Unloadable { row, .. } | ArmedFault::Unevaluable { row, .. } => {
                row.mode().enforces()
            }
        }
    }

    /// The armed id this fault is about, or `None` for a whole-artifact fault
    /// (which is about the law rather than about any one rule).
    #[must_use]
    pub fn id(&self) -> Option<&RuleId> {
        match self {
            ArmedFault::Missing { .. }
            | ArmedFault::Corrupt { .. }
            | ArmedFault::Disarmed { .. } => None,
            ArmedFault::Red(red) => Some(red.row().id()),
            ArmedFault::Unloadable { row, .. } | ArmedFault::Unevaluable { row, .. } => {
                Some(row.id())
            }
        }
    }
}

impl std::fmt::Display for ArmedFault {
    /// The operator's teaching text — the one renderer. Each fault says what is
    /// wrong, why it is not being ignored, and what repairs it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArmedFault::Missing { path } => write!(
                f,
                "this workspace has been armed, but its attested armed-set artifact `{path}` \
                 cannot be read — the law is unreadable, so nothing may be assumed about it. \
                 Restore the artifact, or re-arm to attest the law afresh"
            ),
            ArmedFault::Corrupt { path, detail } => write!(
                f,
                "the attested armed-set artifact `{path}` is corrupt: {detail}. A corrupt \
                 artifact never reads as `nothing armed` — repair the page or re-arm"
            ),
            ArmedFault::Disarmed { path } => write!(
                f,
                "the attested armed-set artifact `{path}` attests no row at all on a workspace \
                 that has been armed. An armed workspace attests SOMETHING: a rule deliberately \
                 not enforced is a row spelled `off`, so zero rows is the absence of attestation \
                 rather than an attestation of absence — the shape a row deletion leaves behind. \
                 Restore the rows, or arm the rules `off` to attest that nothing enforces"
            ),
            ArmedFault::Red(red) => red.fmt(f),
            ArmedFault::Unloadable { row, error } => write!(
                f,
                "`{id}` is armed against `{page}`, whose bytes are the attested bytes but do not \
                 load as the rule they register: {error}",
                id = row.id(),
                page = row.page(),
            ),
            ArmedFault::Unevaluable { row, detail } => write!(
                f,
                "`{id}` is armed against `{page}` and loaded, but its law could not be evaluated \
                 over this change: {detail}. A law that cannot complete never reads as a pass — \
                 repair the rule, or disarm it",
                id = row.id(),
                page = row.page(),
            ),
        }
    }
}

/// One armed rule governing a path: the row that attests it, and the page it
/// loaded into. Construction is sealed to [`resolve_armed_law`], so a rule in hand
/// passed selection, the freeze check, and its own load gate.
#[derive(Debug, Clone)]
pub struct ArmedRule {
    row: ArmedRow,
    rule: Rule,
}

impl ArmedRule {
    /// The attested row — its id, page, pinned rev, arm root, and mode.
    #[must_use]
    pub fn row(&self) -> &ArmedRow {
        &self.row
    }

    /// The loaded rule page.
    #[must_use]
    pub fn rule(&self) -> &Rule {
        &self.rule
    }

    /// How the row acts. `warn`/`block` is check vocabulary, `armed` is a hook's.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.row.mode()
    }

    /// The rule's id — the name the artifact keys on and every report labels with.
    #[must_use]
    pub fn id(&self) -> &RuleId {
        self.row.id()
    }
}

/// A workspace's armed law as it stands at ONE path: the rules that govern the
/// write, and every fault that kept a rule from governing it.
///
/// Rules and faults are both first-class outputs. A fault never deletes the rules
/// beside it (that was the defect: one bad page silencing a whole path), and a
/// surviving rule never hides the fault next to it.
#[derive(Debug, Clone)]
pub struct ArmedLaw {
    ever_armed: bool,
    rules: Vec<ArmedRule>,
    faults: Vec<ArmedFault>,
}

impl ArmedLaw {
    /// Whether the workspace has NEVER been armed. The gate is then a bit-for-bit
    /// no-op — not "armed with nothing", which is [`ArmedFault::Disarmed`].
    #[must_use]
    pub fn never_armed(&self) -> bool {
        !self.ever_armed
    }

    /// The rules governing this path, each loaded and drift-verified.
    #[must_use]
    pub fn rules(&self) -> &[ArmedRule] {
        &self.rules
    }

    /// Every fault, in the order found: whole-artifact first, then per row.
    #[must_use]
    pub fn faults(&self) -> &[ArmedFault] {
        &self.faults
    }

    /// The faults that must REFUSE this write. Empty is not "no faults" — a hook
    /// row's fault is reported and survived, never refused.
    pub fn refusing(&self) -> impl Iterator<Item = &ArmedFault> {
        self.faults.iter().filter(|fault| fault.refuses())
    }

    /// Every page this workspace ATTESTED at this path, whether or not its row
    /// still stands — the binding law's question.
    ///
    /// Red and unloadable rows are included deliberately: a row is bound by the
    /// ARM act that attested it, and excluding them would make a drifted page
    /// freely editable.
    pub fn armed_pages(&self) -> impl Iterator<Item = &str> {
        self.rules
            .iter()
            .map(|rule| rule.row.page())
            .chain(self.faults.iter().filter_map(|fault| match fault {
                ArmedFault::Red(red) => Some(red.row().page()),
                ArmedFault::Unloadable { row, .. } | ArmedFault::Unevaluable { row, .. } => {
                    Some(row.page())
                }
                ArmedFault::Missing { .. }
                | ArmedFault::Corrupt { .. }
                | ArmedFault::Disarmed { .. } => None,
            }))
    }
}

/// Resolve what governs a write at `at_path`, and everything that stopped a rule
/// from governing it.
///
/// `artifact` is the attested artifact's bytes (`None` when it is absent OR
/// unreadable — neither says what the law is, and the once-armed pivot below makes
/// both fail closed); `ever_armed` is the once-armed marker's presence; `pages`
/// reads pinned pages (injected — `policy` does no I/O); `limits` bound each page's
/// load gate.
///
/// The ladder:
/// 1. `!ever_armed` → never armed, the bit-for-bit no-op. The artifact is not even
///    read: a workspace can only be armed by an attested arm, which sets the
///    marker, so a stray artifact on a never-armed workspace is a stray file.
/// 2. artifact absent/unreadable → [`ArmedFault::Missing`].
/// 3. artifact does not parse → [`ArmedFault::Corrupt`].
/// 4. artifact attests ZERO rows → [`ArmedFault::Disarmed`] (module docs).
/// 5. otherwise select + verify in one call, then load each firing row's page. A
///    row that reddens or will not load is ONE fault; the rows beside it still
///    govern.
#[must_use]
pub fn resolve_armed_law(
    artifact: Option<&str>,
    ever_armed: bool,
    at_path: &str,
    pages: &dyn PageSource,
    limits: CheckLimits,
) -> ArmedLaw {
    if !ever_armed {
        return ArmedLaw {
            ever_armed: false,
            rules: Vec::new(),
            faults: Vec::new(),
        };
    }
    let broken = |fault: ArmedFault| ArmedLaw {
        ever_armed: true,
        rules: Vec::new(),
        faults: vec![fault],
    };
    let path = || ARMED_RULES_PATH.to_string();

    let Some(page) = artifact else {
        return broken(ArmedFault::Missing { path: path() });
    };
    let artifact = match parse_artifact(page) {
        Ok(artifact) => artifact,
        Err(corrupt) => {
            return broken(ArmedFault::Corrupt {
                path: path(),
                detail: corrupt.detail,
            });
        }
    };
    if artifact.rows().is_empty() {
        return broken(ArmedFault::Disarmed { path: path() });
    }

    // One call — `verify_at` composes selection and the freeze check in the
    // correct order. Red rows are faults here, not silent absences.
    let verdict = artifact.verify_at(at_path, pages);
    let mut faults: Vec<ArmedFault> = verdict.red().iter().cloned().map(ArmedFault::Red).collect();
    let mut rules = Vec::with_capacity(verdict.firing().len());
    for row in verdict.firing() {
        match load_row(row, pages, limits) {
            Ok(rule) => rules.push(ArmedRule {
                row: row.clone(),
                rule,
            }),
            Err(fault) => faults.push(*fault),
        }
    }
    ArmedLaw {
        ever_armed: true,
        rules,
        faults,
    }
}

/// Load one firing row's pinned page into a [`Rule`], or say why it could not be.
///
/// Loads through [`load_rule`] — the one enforcement point for the kind seam.
/// The filename-shaped leg loaders still assert a `kind:` key tag registration
/// replaced, so calling one directly would let a page in the ruled shape arm and
/// never fire.
///
/// The layer is `Workspace` because an armed row's `page` is a workspace path.
fn load_row(
    row: &ArmedRow,
    pages: &dyn PageSource,
    limits: CheckLimits,
) -> Result<Rule, Box<ArmedFault>> {
    let red = |why: Redness| Box::new(ArmedFault::Red(RedRow::new(row.clone(), why)));

    // A miss here is the page vanishing mid-write — the same redness `verify_at`
    // would have found one call earlier.
    let bytes = pages.read(row.page()).map_err(|e| {
        red(Redness::Missing {
            detail: e.to_string(),
        })
    })?;
    // Reaching the else arm means the page stopped being a rule page between the
    // two reads; verification would have reddened it the same way.
    let Ok(Some(registration)) = register_page(PageRef {
        layer: ScopeLayer::Workspace,
        page: row.page(),
        bytes: &bytes,
    }) else {
        return Err(red(Redness::mode_outside_kind(Vec::new())));
    };
    load_rule(&registration, &bytes, limits).map_err(|error| {
        Box::new(ArmedFault::Unloadable {
            row: row.clone(),
            error,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::armed::{ArmRequest, ArmRoot, ArmedArtifact, arm};
    use crate::registration::{RuleIndex, page_rev};
    use std::collections::BTreeMap;

    /// The pinned pages, as an in-memory `path → bytes` map.
    struct MemPages(BTreeMap<String, String>);

    impl PageSource for MemPages {
        fn read(&self, page: &str) -> std::io::Result<String> {
            self.0.get(page).cloned().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, format!("no {page}"))
            })
        }
    }

    /// A CHECK page: registration tag + id, `paths:` scope, and the law's entry
    /// point. No `kind:` key — the tag is the one name (ruling § 1).
    fn check_page(id: &str) -> String {
        format!(
            "---\ntags: [type/rule, rules/check]\nid: {id}\npaths:\n  - tasks/**\n---\n\n\
             # {id}\n\n```starlark\ndef check_change(change):\n    pass\n```\n"
        )
    }

    /// A HOOK page in the RULED shape — the tag names the leg and no `kind:` key
    /// restates it.
    fn hook_page(id: &str) -> String {
        format!(
            "---\ntags: [type/rule, rules/hook]\nid: {id}\nseverity: info\n\
             paths: [\"tasks/*.md\"]\ncaps:  [proto.send]\n\
             budget: {{ steps: 10000, mem: 4194304 }}\nhow:\n  route: {{ info: channel-review }}\n\
             ---\n\n```starlark\ndef on_change(event):\n    pass\n```\n"
        )
    }

    /// Arm every `(page path, bytes, id, mode)` at the workspace root, and hand back
    /// the rendered artifact plus a page source over the same bytes.
    fn armed(pages: &[(&str, String, &str, Mode)]) -> (String, MemPages) {
        let index = RuleIndex::discover(pages.iter().map(|(path, bytes, ..)| PageRef {
            layer: ScopeLayer::Workspace,
            page: path,
            bytes,
        }));
        let source = MemPages(
            pages
                .iter()
                .map(|(path, bytes, ..)| ((*path).to_string(), bytes.clone()))
                .collect(),
        );
        let artifact = arm(
            &index,
            &ArmRoot::workspace(),
            pages.iter().map(|(_, bytes, id, mode)| ArmRequest {
                id: RuleId::parse(id).expect("a legal id"),
                mode: *mode,
                attested_rev: page_rev(bytes),
            }),
            &source,
            CheckLimits::default(),
        )
        .expect("the fixture arms");
        (artifact.render(), source)
    }

    fn resolve(artifact: Option<&str>, ever_armed: bool, pages: &MemPages) -> ArmedLaw {
        resolve_armed_law(
            artifact,
            ever_armed,
            "tasks/x.md",
            pages,
            CheckLimits::default(),
        )
    }

    fn no_pages() -> MemPages {
        MemPages(BTreeMap::new())
    }

    // ── the once-armed pivot ──────────────────────────────────────────────────

    #[test]
    fn a_never_armed_workspace_is_a_no_op_and_the_artifact_is_not_even_read() {
        let law = resolve(Some("garbage that is not an artifact"), false, &no_pages());
        assert!(law.never_armed());
        assert!(law.rules().is_empty());
        assert!(law.faults().is_empty(), "a stray artifact is a stray file");
    }

    #[test]
    fn an_absent_artifact_on_a_once_armed_workspace_refuses() {
        let law = resolve(None, true, &no_pages());
        assert!(!law.never_armed());
        let [ArmedFault::Missing { path }] = law.faults() else {
            panic!("expected one Missing fault: {:?}", law.faults());
        };
        assert_eq!(path, ARMED_RULES_PATH);
        assert_eq!(law.refusing().count(), 1, "the law is unreadable");
    }

    #[test]
    fn a_corrupt_artifact_refuses_and_never_reads_as_nothing_armed() {
        let (page, pages) = armed(&[("check.md", check_page("a.check"), "a.check", Mode::Block)]);
        let corrupt = page.replace("| id | page | rev | scope | mode |", "| id | page |");
        let law = resolve(Some(&corrupt), true, &pages);
        let [ArmedFault::Corrupt { detail, .. }] = law.faults() else {
            panic!("expected one Corrupt fault: {:?}", law.faults());
        };
        assert!(detail.contains("column header"), "names it: {detail}");
        assert!(law.rules().is_empty(), "a corrupt law arms nothing");
        assert_eq!(law.refusing().count(), 1);
    }

    // ── F2: the row-deletion disarm ───────────────────────────────────────────

    /// F2: deleting every row leaves a well-formed artifact page — title,
    /// preamble, § 4 header, zero data rows — which must refuse rather than
    /// read as a silent total disarm.
    #[test]
    fn a_well_formed_empty_artifact_on_a_once_armed_workspace_refuses_instead_of_disarming() {
        let empty = ArmedArtifact::default().render();
        parse_artifact(&empty).expect("the deletion is well-formed, not corrupt");

        let law = resolve(Some(&empty), true, &no_pages());
        let [ArmedFault::Disarmed { path }] = law.faults() else {
            panic!("expected one Disarmed fault: {:?}", law.faults());
        };
        assert_eq!(path, ARMED_RULES_PATH);
        assert_eq!(
            law.refusing().count(),
            1,
            "zero rows is the absence of attestation, so the door fails closed"
        );
        assert!(
            law.faults()[0].to_string().contains("spelled `off`"),
            "the teaching names the legitimate disarm: {}",
            law.faults()[0]
        );
    }

    /// An attested-`off` row IS an attestation: a workspace that deliberately
    /// enforces nothing gates nothing and faults not at all.
    #[test]
    fn rows_attested_off_are_a_legitimate_disarm_and_never_a_fault() {
        let (page, pages) = armed(&[("check.md", check_page("a.check"), "a.check", Mode::Off)]);
        let law = resolve(Some(&page), true, &pages);
        assert!(law.faults().is_empty(), "{:?}", law.faults());
        assert!(law.rules().is_empty(), "an off row never fires");
    }

    // ── F-1: one bad row must not silence the rules beside it ─────────────────

    /// F-1: two rules govern the path; one page registers but does not load.
    /// The faulting row is reported by name and the other rule still governs.
    ///
    /// The bad row is HAND-WRITTEN at the page's true rev, because `arm` no
    /// longer produces one: the ARM act loads every firing winner and refuses
    /// `ArmFault::Unloadable` (card `mrd-arm-loads-page`). The fault stays
    /// reachable — a hand-edited artifact, a row armed by an older engine, or
    /// limits that differ between arm and fire — so the fire path must still
    /// fail closed on it, which is what this test holds.
    #[test]
    fn one_unloadable_row_does_not_silence_the_rules_beside_it() {
        // Registers by tag but declares no hook — the two layers.
        let bare = "---\ntags: [type/rule, rules/hook]\nid: bad.hook\n---\n\n# rule\n".to_string();
        let (armed_page, mut pages) =
            armed(&[("good.md", hook_page("good.hook"), "good.hook", Mode::Armed)]);
        pages.0.insert("bad.md".to_string(), bare.clone());
        let page = format!(
            "{armed_page}| `bad.hook` | `bad.md` | `{rev}` | `.` | `armed` |\n",
            rev = page_rev(&bare)
        );
        let law = resolve(Some(&page), true, &pages);

        let [ArmedFault::Unloadable { row, .. }] = law.faults() else {
            panic!("expected one Unloadable fault: {:?}", law.faults());
        };
        assert_eq!(row.id().as_str(), "bad.hook");
        assert_eq!(law.rules().len(), 1, "the good rule still governs");
        assert_eq!(law.rules()[0].id().as_str(), "good.hook");
        assert_eq!(
            law.refusing().count(),
            0,
            "a hook's fault is reported and survived — a reaction never vetoes"
        );
    }

    // ── the kind split, applied to every fault ────────────────────────────────

    #[test]
    fn a_red_check_row_refuses_where_a_red_hook_row_only_reports() {
        for (mode, page, refusing) in [
            (Mode::Block, check_page("a.check"), 1),
            (Mode::Armed, hook_page("a.hook"), 0),
        ] {
            let id = if mode == Mode::Block {
                "a.check"
            } else {
                "a.hook"
            };
            let (artifact, mut pages) = armed(&[("p.md", page.clone(), id, mode)]);
            // Edit the pinned page after arming: the row reddens.
            pages
                .0
                .insert("p.md".to_string(), format!("{page}\n<!-- moved -->\n"));
            let law = resolve(Some(&artifact), true, &pages);
            assert!(
                matches!(law.faults(), [ArmedFault::Red(_)]),
                "{mode:?}: {:?}",
                law.faults()
            );
            assert_eq!(law.refusing().count(), refusing, "{mode:?}");
            assert!(law.rules().is_empty(), "{mode:?}: a red row never fires");
        }
    }

    /// A page in the ruled shape — tag, id, no `kind:` — must load; the
    /// filename-shaped hook loader still asserts `kind: hook`.
    #[test]
    fn a_hook_page_in_the_ruled_shape_loads_without_a_kind_key() {
        let page = hook_page("a.hook");
        assert!(!page.contains("kind:"), "the ruled shape declares no kind:");
        let (artifact, pages) = armed(&[("p.md", page, "a.hook", Mode::Armed)]);
        let law = resolve(Some(&artifact), true, &pages);
        assert!(law.faults().is_empty(), "{:?}", law.faults());
        assert_eq!(law.rules().len(), 1);
        assert!(
            law.rules()[0].rule().hook().is_some(),
            "the hook leg loaded"
        );
    }

    /// Selection is by ARM ROOT; a rule's `paths:` scope is the consumer's
    /// filter — folding `paths:` in here would make an out-of-scope write look
    /// unarmed and hide the faults governing it.
    #[test]
    fn selection_is_by_arm_root_and_declared_scope_stays_the_consumers() {
        let (artifact, pages) =
            armed(&[("check.md", check_page("a.check"), "a.check", Mode::Block)]);
        let law = resolve_armed_law(
            Some(&artifact),
            true,
            "notes/x.md",
            &pages,
            CheckLimits::default(),
        );
        assert_eq!(law.rules().len(), 1, "the workspace root contains the path");
        assert!(
            !law.rules()[0].rule().matches_path("notes/x.md"),
            "and the rule's own scope excludes it"
        );
        assert!(law.faults().is_empty());
    }
}
