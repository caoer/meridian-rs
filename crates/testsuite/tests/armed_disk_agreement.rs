//! **The two copies of the armed-plane disk edge must answer identically.**
//!
//! # Why this test exists
//! `wire_serve::armed_disk` and `run::gate` each read the once-armed marker, the attested
//! artifact, and the pinned pages. They are two copies on purpose: `run`'s charter
//! forbids it from naming `wire-serve` (that would drag the wire, the daemon, and the
//! render plane into the run plane), and hoisting a shared home was ruled out of the
//! cutover diff and re-owed with the ARM writer.
//!
//! But **two readers of one marker is the exact defect class the cutover ruling is
//! about.** While the write door read `conventions/INDEX.md` and the reaction feeder read
//! the artifact, one workspace could be armed for one surface and never-armed for the
//! other; the measured symptom was every write refused, plus two hung tests waiting for a
//! notification a refused write never produced. A comment promising the copies agree is
//! what that state looked like beforehand.
//!
//! So the copies are pinned against each other here, in the crate that may name both. The
//! matrix is the pivot's whole input space — marker × artifact — and each cell asserts
//! the two readers reach the SAME disposition, not merely that each is individually
//! plausible.
//!
//! This test is the drift window's replacement, and it is load-bearing until
//! `[[arm-disk-edge]]` decides the one reader home. If it is ever deleted, the
//! duplication silently becomes unguarded again.

use std::path::Path;

/// Every state the once-armed pivot can be in: the marker present or not, crossed
/// with the artifact absent, well-formed-and-arming, well-formed-and-EMPTY (the
/// row-deletion disarm), and corrupt.
const ARTIFACTS: &[(&str, Option<&str>)] = &[
    ("absent", None),
    ("corrupt", Some("not an artifact at all\n")),
];

/// A rule page that registers by tag and loads — the arming leg's subject.
const RULE_PAGE: &str = "---\ntags: [type/rule, rules/check]\nid: agree.check\n\
    paths:\n  - tasks/**\n---\n\n# agree.check\n\n\
    ```starlark\ndef check_change(change):\n    pass\n```\n";

const RULE_PATH: &str = "rules/agree.md";

fn write(root: &Path, rel: &str, body: &str) {
    let full = root.join(rel);
    std::fs::create_dir_all(full.parent().expect("parent")).expect("dir");
    std::fs::write(full, body).expect("write");
}

/// The artifact arming `agree.check` at the workspace root, pinned to the live rev.
fn arming_artifact() -> String {
    let index = policy::RuleIndex::discover([policy::PageRef {
        layer: policy::ScopeLayer::Workspace,
        page: RULE_PATH,
        bytes: RULE_PAGE,
    }]);
    policy::armed::arm(
        &index,
        &policy::armed::ArmRoot::workspace(),
        [policy::armed::ArmRequest {
            id: policy::RuleId::parse("agree.check").expect("a legal id"),
            mode: policy::armed::Mode::Block,
            attested_rev: policy::page_rev(RULE_PAGE),
        }],
    )
    .expect("arms")
    .render()
}

/// A comparable summary of one reader's answer. Comparing the whole disposition —
/// not just "did it fault" — is what makes a divergence in WHICH fault, or in how
/// many rules survived it, a failure rather than a coincidence.
fn disposition(law: &policy::ArmedLaw) -> String {
    format!(
        "never_armed={} rules=[{}] faults=[{}] refusing={}",
        law.never_armed(),
        law.rules()
            .iter()
            .map(|r| r.id().as_str().to_string())
            .collect::<Vec<_>>()
            .join(","),
        law.faults()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" | "),
        law.refusing().count(),
    )
}

#[test]
fn the_two_armed_disk_edges_agree_on_every_pivot_state() {
    let arming = arming_artifact();
    let emptied = policy::armed::ArmedArtifact::default().render();

    let mut artifacts: Vec<(&str, Option<&str>)> = ARTIFACTS.to_vec();
    artifacts.push(("arming", Some(&arming)));
    // The row-deletion disarm: well-formed, parses clean, attests nothing.
    artifacts.push(("well-formed-but-empty", Some(&emptied)));

    let mut checked = 0;
    for marker in [false, true] {
        for (name, artifact) in &artifacts {
            let dir = tempfile::tempdir().expect("tempdir");
            let root_path = dir.path();
            write(root_path, RULE_PATH, RULE_PAGE);
            if let Some(body) = artifact {
                write(root_path, fs::domain::ARMED_RULES_PATH, body);
            }
            if marker {
                write(root_path, fs::domain::ATTESTED_MARKER_PATH, "");
            }
            let root = fs::WorkspaceRoot(root_path.to_path_buf());

            let from_wire_serve = wire_serve::armed_disk::resolve_at(&root, "tasks/x.md");
            let from_run = run::gate::resolve_at(&root, "tasks/x.md");

            assert_eq!(
                disposition(&from_wire_serve),
                disposition(&from_run),
                "the two armed-disk edges disagree at marker={marker} artifact={name} — one \
                 workspace, two answers about what governs it. That divergence is what made a \
                 whole cutover ruling necessary; it must not come back through a copy."
            );

            // Each reader's marker probe is the pivot itself, so pin it directly
            // too: a matching disposition could otherwise hide two wrong answers.
            assert_eq!(
                wire_serve::armed_disk::once_armed(&root),
                run::gate::once_armed(&root),
                "the two once-armed probes disagree at marker={marker}"
            );
            assert_eq!(
                wire_serve::armed_disk::read_artifact(&root).is_some(),
                run::gate::read_artifact(&root).is_some(),
                "the two artifact readers disagree on presence at artifact={name}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 8, "the whole marker x artifact matrix is driven");
}

/// The control the matrix above needs: the states must be DISTINGUISHABLE, or two
/// readers that always answered "never armed" would agree perfectly and prove
/// nothing. This asserts the pivot really moves.
#[test]
fn the_pivot_states_are_distinguishable_so_agreement_is_not_vacuous() {
    let arming = arming_artifact();
    let mut seen = std::collections::BTreeSet::new();
    for (marker, artifact) in [
        (false, Some(arming.as_str())),
        (true, None),
        (true, Some(arming.as_str())),
        (true, Some("not an artifact at all\n")),
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        write(dir.path(), RULE_PATH, RULE_PAGE);
        if let Some(body) = artifact {
            write(dir.path(), fs::domain::ARMED_RULES_PATH, body);
        }
        if marker {
            write(dir.path(), fs::domain::ATTESTED_MARKER_PATH, "");
        }
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        seen.insert(disposition(&wire_serve::armed_disk::resolve_at(
            &root,
            "tasks/x.md",
        )));
    }
    assert_eq!(
        seen.len(),
        4,
        "the four states must read differently, or the agreement test is vacuous: {seen:#?}"
    );
}
