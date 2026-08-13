//! The mount table's gates.
//!
//! Every refusal is asserted beside its acceptance. Environment preconditions
//! fail loudly: where a directory is required it is `expect`ed with the
//! reason, never skipped — a skipped ceiling gate is a false negative that
//! agrees with the failure it exists to detect.

use std::path::{Path, PathBuf};

use super::*;

/// Wrap mount blocks in a well-formed config.
fn config_of(blocks: &[String]) -> String {
    format!(
        "---\ntype: meridian-config\nversion: 1\n---\n\n# My system\n\n{}",
        blocks.join("\n")
    )
}

/// One `meridian-mount` block, in canonical field order.
fn mount_block(name: &str, path: &Path, vault: Option<&str>, pin: Option<&str>) -> String {
    use std::fmt::Write as _;
    let mut block = format!(
        "```meridian-mount\nname: {name}\npath: {}\n",
        path.display()
    );
    if let Some(vault) = vault {
        let _ = writeln!(block, "vault: {vault}");
    }
    if let Some(pin) = pin {
        let _ = writeln!(block, "pin: {pin}");
    }
    block.push_str("```\n");
    block
}

/// A vault block whose vault name equals its canonical name.
fn vault_block(name: &str, path: &Path) -> String {
    mount_block(name, path, Some(name), None)
}

fn parse_config(raw: &str) -> Config {
    crate::parse(raw, Path::new("~/MERIDIAN.md")).expect("the fixture must parse — U6's gate")
}

fn table(raw: &str) -> Result<MountTable, MountError> {
    bind(&parse_config(raw))
}

fn refuse(raw: &str) -> MountError {
    table(raw).expect_err("this config must refuse to bind")
}

/// Write a root's self-declaration into `dir`.
fn declare(dir: &Path, name: &str) {
    declare_raw(
        dir,
        &format!("---\ntype: {DECLARATION_TYPE}\nversion: 1\nname: {name}\n---\n\n# {name}\n"),
    );
}

fn declare_raw(dir: &Path, raw: &str) {
    std::fs::write(dir.join(DECLARATION_FILENAME), raw).expect("write the root declaration");
}

/// A temp directory that the ceiling admits — asserted, because a sandbox the
/// ceiling happens to refuse would make every acceptance here vacuous.
fn root(name: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(name);
    std::fs::create_dir_all(&path).expect("create the root");
    let canonical = std::fs::canonicalize(&path).expect("canonicalize the root");
    assert_eq!(
        workspace::deny_reason(&canonical),
        None,
        "the sandbox itself must pass the ceiling, or every acceptance below is vacuous"
    );
    (dir, path)
}

/// An existing directory the ceiling refuses, with the reason it refuses for.
/// `expect`s rather than skips (see the module note).
fn denied_dir(what: &str, path: PathBuf) -> PathBuf {
    let reason = workspace::deny_reason(&path).unwrap_or_else(|| {
        panic!(
            "precondition: {} ({}) must be refused by the deny ceiling; the ceiling gate cannot run otherwise",
            what,
            path.display()
        )
    });
    assert!(
        path.is_dir(),
        "precondition: {what} ({}) must exist as a directory — {reason} is only reachable through a canonicalizable path",
        path.display()
    );
    path
}

fn home() -> PathBuf {
    let home = std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .expect("precondition: HOME must be set for the ceiling gate");
    denied_dir("$HOME", PathBuf::from(home))
}

// ---------------------------------------------------------------------------
// Gate 1 — THE CEILING, proven negatively, and the whole parse fails
// ---------------------------------------------------------------------------

/// Every ceiling class refuses, and each refusal fails the whole bind. The
/// classes are `workspace::DenyReason`'s own, reached through real paths
/// rather than a stub.
#[test]
fn every_ceiling_class_refuses_the_whole_bind() {
    let cache_root = workspace::cache_root().expect("precondition: a resolvable cache root");
    let xdg = ["XDG_CACHE_HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME"]
        .iter()
        .find_map(|key| std::env::var_os(key).map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .expect("precondition: an XDG base directory to point at");

    let cases: Vec<(&str, PathBuf)> = vec![
        ("$HOME", home()),
        ("the filesystem root", denied_dir("/", PathBuf::from("/"))),
        ("/tmp", denied_dir("/tmp", PathBuf::from("/tmp"))),
        ("an XDG base dir", denied_dir("an XDG base dir", xdg)),
        (
            "the cache root",
            denied_dir("the meridian cache root", cache_root),
        ),
    ];

    for (what, path) in cases {
        let err = refuse(&config_of(&[vault_block("poison", &path)]));
        assert_eq!(
            err.reason,
            MountReason::MountPathDenied,
            "{what} must refuse as mount-path-denied"
        );
        let text = err.to_string();
        assert!(
            text.contains(NO_PARTIAL_LOAD_CLAUSE),
            "{what}: the refusal must say nothing loaded — {text}"
        );
        assert!(
            text.contains("Fix: "),
            "{what}: the refusal must teach the fix — {text}"
        );
    }
}

/// No partial table survives a refused mount: a legal root is declared first
/// and the poisoned one second, so a build that bound as it went would have a
/// one-entry table to hand back. It cannot: `bind` returns `Err`.
#[test]
fn a_refused_mount_leaves_no_partial_table() {
    let (_keep, good) = root("wiki");
    declare(&good, "wiki");

    let raw = config_of(&[vault_block("wiki", &good), vault_block("poison", &home())]);

    let err = refuse(&raw);
    assert_eq!(err.reason, MountReason::MountPathDenied);
    assert!(
        err.detail.contains("poison"),
        "the refusal names the offending mount, not the innocent one: {}",
        err.detail
    );
    assert!(
        !err.detail.contains("`wiki`"),
        "the refusal must not implicate the legal mount: {}",
        err.detail
    );
    assert!(
        table(&raw).is_err(),
        "there is no value to observe: bind is MountTable's only constructor"
    );

    // The acceptance half: the legal root alone binds, so the refusal above is
    // the ceiling biting, not a broken fixture.
    let clean = table(&config_of(&[vault_block("wiki", &good)])).expect("the legal root binds");
    assert_eq!(clean.mounts().len(), 1);
    assert_eq!(clean.mounts()[0].state(), &MountState::Bound);
}

// ---------------------------------------------------------------------------
// Gate 2 — canonicalize at bind: the symlink, the trailing slash, the nesting
// ---------------------------------------------------------------------------

/// A symlinked spelling and the real path are one tree. Bound under two names
/// they would yield two canonical refs over identical bytes, which the
/// read-mint recheck cannot tell apart.
#[test]
fn a_symlink_cannot_smuggle_one_tree_in_twice() {
    let (_keep, real) = root("field-notes");
    declare(&real, "field-notes");
    let link = real.parent().expect("parent").join("repos-field-notes");
    std::os::unix::fs::symlink(&real, &link).expect("symlink the root");

    // The topology is the measured one before anything is asserted about it.
    assert_eq!(
        std::fs::canonicalize(&link).expect("canonicalize the symlink"),
        std::fs::canonicalize(&real).expect("canonicalize the real path"),
        "the fixture must reproduce the measured symlink topology"
    );

    let err = refuse(&config_of(&[
        vault_block("field-notes", &real),
        mount_block("repos", &link, Some("repos"), None),
    ]));
    assert_eq!(err.reason, MountReason::DuplicateMountPath);
    assert!(
        err.detail.contains("`repos`") && err.detail.contains("`field-notes`"),
        "both spellings are named: {}",
        err.detail
    );
}

/// The trailing-slash half of the measured case. `Path` equality is
/// component-wise, so this passes even without canonicalization — the symlink
/// case is what canonicalization buys.
#[test]
fn a_trailing_slash_is_the_same_tree() {
    let (_keep, real) = root("field-notes");
    declare(&real, "field-notes");
    let slashed = PathBuf::from(format!("{}/", real.display()));

    let err = refuse(&config_of(&[
        vault_block("field-notes", &real),
        mount_block("env", &slashed, Some("env"), None),
    ]));
    assert_eq!(err.reason, MountReason::DuplicateMountPath);
}

/// Equal-or-nested, and its acceptance half: `/a/wiki` + `/a/wiki/sub`
/// refuses; `/a/wiki` + `/a/wiki-two` binds — the prefix test is a
/// path-segment boundary.
#[test]
fn nesting_is_refused_and_the_sibling_is_not() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outer = dir.path().join("wiki");
    let inner = outer.join("sub");
    let sibling = dir.path().join("wiki-two");
    std::fs::create_dir_all(&inner).expect("create the inner root");
    std::fs::create_dir_all(&sibling).expect("create the sibling root");
    declare(&outer, "wiki");
    declare(&inner, "sub");
    declare(&sibling, "wiki-two");

    let err = refuse(&config_of(&[
        vault_block("wiki", &outer),
        vault_block("sub", &inner),
    ]));
    assert_eq!(err.reason, MountReason::NestedMount);

    // ...and the outer-declared-second direction refuses too: containment is a
    // relation, not an ordering.
    let reversed = refuse(&config_of(&[
        vault_block("sub", &inner),
        vault_block("wiki", &outer),
    ]));
    assert_eq!(reversed.reason, MountReason::NestedMount);

    // The acceptance half (row M5).
    let bound = table(&config_of(&[
        vault_block("wiki", &outer),
        vault_block("wiki-two", &sibling),
    ]))
    .expect("a sibling root is not a nested root");
    assert_eq!(bound.mounts().len(), 2);
    assert!(bound.is_clear(), "both siblings bind: {:?}", bound.mounts());
}

// ---------------------------------------------------------------------------
// Gate 3 — declared vs bound: BOTH arms
// ---------------------------------------------------------------------------

/// The refusal arm: the root declares `wiki`, the table binds `field-notes`.
/// Fails loud, naming both spellings.
#[test]
fn a_declared_bound_mismatch_fails_loud_naming_both_spellings() {
    let (_keep, dir) = root("field-notes");
    declare(&dir, "wiki");

    let err = refuse(&config_of(&[vault_block("field-notes", &dir)]));
    assert_eq!(err.reason, MountReason::DeclaredBoundMismatch);
    let text = err.to_string();
    assert!(
        text.contains("`field-notes`") && text.contains("`wiki`"),
        "both spellings must appear: {text}"
    );
    assert!(
        text.contains(NO_PARTIAL_LOAD_CLAUSE),
        "a mismatch fails the whole table: {text}"
    );
}

/// The acceptance arm: a matching pair binds, and the mount reports the name
/// the root declared — the check is reading the declaration, not merely
/// failing to find a disagreement.
#[test]
fn a_matching_declaration_binds() {
    let (_keep, dir) = root("field-notes");
    declare(&dir, "field-notes");

    let bound =
        table(&config_of(&[vault_block("field-notes", &dir)])).expect("a matching pair binds");
    let mount = &bound.mounts()[0];
    assert_eq!(mount.state(), &MountState::Bound);
    assert_eq!(mount.declared_name(), Some("field-notes"));
    assert!(bound.is_clear());
}

// ---------------------------------------------------------------------------
// Gate 4 — an absent declaration renders GREY, named
// ---------------------------------------------------------------------------

/// The absent case: grey, with the missing declaration named — not red, not a
/// `file_not_found`. It refuses, and the table stays loaded.
#[test]
fn an_absent_declaration_renders_grey_naming_the_missing_file() {
    let (_keep, dir) = root("field-notes");
    // Deliberately no declaration written.

    let bound = table(&config_of(&[vault_block("field-notes", &dir)]))
        .expect("an undeclared root is not a parse failure");
    let mount = &bound.mounts()[0];

    assert_eq!(mount.state().word(), "grey(undeclared)");
    assert!(mount.state().refuses(), "grey refuses (S3-R6)");
    assert!(!bound.is_clear());
    assert_eq!(mount.declared_name(), None);

    let detail = mount.state().detail();
    assert!(
        detail.contains(DECLARATION_FILENAME) && detail.contains(&dir.display().to_string()),
        "the missing declaration is named by path: {detail}"
    );
    assert!(
        !detail.contains("file_not_found") && !mount.state().word().starts_with("red"),
        "not red, not file_not_found: {} / {detail}",
        mount.state().word()
    );
}

/// A declaration that is present but does not read is neither absent nor a
/// mismatch: it greys with its own word and does not fail this machine's
/// whole parse.
#[test]
fn an_unreadable_declaration_greys_rather_than_failing_the_table() {
    let (_keep, dir) = root("field-notes");
    declare_raw(&dir, "# just a page, no frontmatter\n");

    let bound = table(&config_of(&[vault_block("field-notes", &dir)]))
        .expect("a foreign root's broken content is not this machine's parse failure");
    assert_eq!(
        bound.mounts()[0].state().word(),
        "grey(declaration-unreadable)"
    );

    // A declaration whose `type:` names something else is the same class.
    declare_raw(&dir, "---\ntype: meridian-config\nversion: 1\n---\n");
    let bound = table(&config_of(&[vault_block("field-notes", &dir)])).expect("still not a failure");
    assert_eq!(
        bound.mounts()[0].state().word(),
        "grey(declaration-unreadable)"
    );
    assert!(
        bound.mounts()[0]
            .state()
            .detail()
            .contains(DECLARATION_TYPE),
        "the detail names the type it wanted: {}",
        bound.mounts()[0].state().detail()
    );
}

// ---------------------------------------------------------------------------
// Gate 5 — mount-as-claim, end to end, asserting a STATE CHANGE (R40)
// ---------------------------------------------------------------------------

/// The claim works both ways in one run: a mount pins the root it declares
/// and binds, then the root's declaration is edited and the same config
/// reddens. The two verdicts are compared to each other, and the `live` token
/// the red carries is asserted to be the declaration's real new fingerprint.
#[test]
fn a_mount_pins_the_root_it_declares_and_drift_reddens_it() {
    let (_keep, dir) = root("field-notes");
    declare(&dir, "field-notes");

    let pin = declaration_fingerprint(&dir);
    let raw = config_of(&[mount_block(
        "field-notes",
        &dir,
        Some("field-notes"),
        Some(&pin),
    )]);

    let before = table(&raw).expect("bind").mounts()[0].state().clone();
    assert_eq!(before, MountState::Bound, "the claim verifies as minted");

    // Drift the declaration, keeping the declared name identical — so what
    // reddens is the claim and not the name check.
    let declaration = dir.join(DECLARATION_FILENAME);
    let edited = format!(
        "{}\nAn out-of-band edit to the root's own declaration.\n",
        std::fs::read_to_string(&declaration).expect("read the declaration")
    );
    std::fs::write(&declaration, edited).expect("edit the declaration");

    let after = table(&raw).expect("bind again").mounts()[0].state().clone();
    assert_ne!(
        before, after,
        "R40: the verdict must CHANGE, not merely exist"
    );
    assert_eq!(after.word(), "red(content-drifted)");
    let MountState::Drifted { pinned, live } = &after else {
        panic!("expected a drifted claim, got {after:?}");
    };
    assert_eq!(pinned, &pin);
    assert_eq!(
        live,
        &declaration_fingerprint(&dir),
        "the red carries the live fingerprint — the re-pin candidate"
    );
    assert_eq!(
        table(&raw).expect("bind").mounts()[0].declared_name(),
        Some("field-notes"),
        "the name still matches: the red is the CLAIM's, not the name check's"
    );
}

/// A pin this build cannot decide renders grey, never green. The token is
/// well-formed — parse already refuses a malformed one — so the only way here
/// is an unimplemented member of the self-describing triple.
#[test]
fn an_undecidable_claim_is_grey_and_never_green() {
    let (_keep, dir) = root("archive");
    declare(&dir, "archive");
    let future = "fp1.rawcid9.b3.40b167ed9b42a2beadb7c441b214efdc93069ef443a1cc2b5ae2ccda4cf03152";

    let bound = table(&config_of(&[mount_block(
        "archive",
        &dir,
        None,
        Some(future),
    )]))
    .expect("an undecidable claim is not a parse failure");
    let state = bound.mounts()[0].state();
    assert_eq!(state.word(), "grey(claim-unverifiable)");
    assert!(state.refuses());
    assert!(
        state.detail().contains("codec"),
        "the grey names WHICH member is unknown: {}",
        state.detail()
    );
}

fn declaration_fingerprint(dir: &Path) -> String {
    let raw = std::fs::read_to_string(dir.join(DECLARATION_FILENAME)).expect("read declaration");
    let doc = model::build(raw.clone(), syntax::parse(&raw));
    model::fingerprint::fingerprint(&doc, &doc.root)
        .expect("the declaration has content to fingerprint")
        .into_string()
}

// ---------------------------------------------------------------------------
// Gate 6 — the map is a map: collisions fail loud
// ---------------------------------------------------------------------------

/// Two roots declaring the same canonical name (INV-1, row T1). Decidable
/// from the bytes alone, so it fires at parse — asserted here anyway so a
/// reader walking the three invariants finds all three.
#[test]
fn two_mounts_with_one_canonical_name_fail_loud() {
    let (_keep, a) = root("a");
    let (_keep_b, b) = root("b");
    let raw = config_of(&[
        vault_block("wiki", &a),
        mount_block("wiki", &b, Some("other"), None),
    ]);

    let err = crate::parse(&raw, Path::new("~/MERIDIAN.md"))
        .expect_err("a name is a key: two values for one key is not a map");
    assert_eq!(err.reason, crate::Reason::DuplicateMountName);
}

/// One root mounted at two paths, the other direction of the same law: two
/// distinct names over one canonicalized tree (INV-2, row T2).
#[test]
fn one_tree_under_two_names_fails_loud() {
    let (_keep, dir) = root("wiki");
    declare(&dir, "wiki");

    let err = refuse(&config_of(&[
        vault_block("wiki", &dir),
        mount_block("wiki-again", &dir, Some("wiki-again"), None),
    ]));
    assert_eq!(err.reason, MountReason::DuplicateMountPath);
}

/// The third leg of the map (INV-3, row T3): a vault name is a key too. Without
/// it, a stored `obsidian://` URI carrying that vault name would name two roots.
#[test]
fn two_mounts_naming_one_obsidian_vault_fail_loud() {
    let (_keep, a) = root("a");
    let (_keep_b, b) = root("b");

    let err = refuse(&config_of(&[
        mount_block("wiki", &a, Some("shared"), None),
        mount_block("sessions", &b, Some("shared"), None),
    ]));
    assert_eq!(err.reason, MountReason::DuplicateVaultName);
    assert!(
        err.detail.contains("shared"),
        "the colliding vault name is named: {}",
        err.detail
    );
}

// ---------------------------------------------------------------------------
// Gate 7 — an unseeable root is grey, and the TABLE STAYS LOADED
// ---------------------------------------------------------------------------

/// Row M6. One root being absent from one machine is the topology working as
/// designed, so it greys that root and nothing else: the other mounts stay
/// bound and the table is returned.
#[test]
fn an_unseeable_mount_path_greys_only_that_root() {
    let (_keep, present) = root("field-notes");
    declare(&present, "field-notes");
    let missing = present.parent().expect("parent").join("not-checked-out");

    let bound = table(&config_of(&[
        vault_block("field-notes", &present),
        mount_block("archive", &missing, None, None),
    ]))
    .expect("an unseeable root is not a parse failure — the table stays loaded");

    assert_eq!(bound.mounts().len(), 2, "the table stays loaded, whole");
    assert_eq!(bound.mounts()[0].state(), &MountState::Bound);
    assert_eq!(bound.mounts()[1].state().word(), "grey(path-unseeable)");
    assert!(bound.mounts()[1].canonical_path().is_none());
    assert_eq!(
        bound.mounts()[1].declared_path(),
        missing.display().to_string(),
        "the unseeable path is named verbatim, as written"
    );
    assert!(!bound.is_clear(), "grey refuses, so the table is not clear");
}

// ---------------------------------------------------------------------------
// The three-way map — the matrix's off-diagonal, exercised in all six directions
// ---------------------------------------------------------------------------

/// The six directed translations of the 3 × 3 matrix, walked. Two are total
/// (name ↔ path) and four are partial over the vault axis; both the answers
/// and the partiality are asserted.
#[test]
fn the_three_way_map_answers_in_all_six_directions() {
    let (_keep, wiki) = root("field-notes");
    let (_keep_arch, archive) = root("archive");
    declare(&wiki, "field-notes");
    declare(&archive, "archive");

    let bound = table(&config_of(&[
        mount_block("field-notes", &wiki, Some("ZT wiki"), None),
        mount_block("archive", &archive, None, None),
    ]))
    .expect("bind");
    let canonical = std::fs::canonicalize(&wiki).expect("canonicalize");

    // name → path, name → vault
    let by_name = bound.by_name("field-notes").expect("name → entry");
    assert_eq!(by_name.canonical_path(), Some(canonical.as_path()));
    assert_eq!(by_name.vault(), Some("ZT wiki"));

    // path → name, path → vault. The lookup canonicalizes its argument, so a
    // symlinked or trailing-slash spelling answers too — the measured case.
    let by_path = bound.by_path(&wiki).expect("path → entry");
    assert_eq!(by_path.name(), "field-notes");
    assert_eq!(by_path.vault(), Some("ZT wiki"));
    assert_eq!(
        bound
            .by_path(Path::new(&format!("{}/", wiki.display())))
            .map(Mount::name),
        Some("field-notes"),
        "a trailing slash is the same tree at LOOKUP too, not only at bind"
    );

    // vault → name, vault → path
    let by_vault = bound.by_vault("ZT wiki").expect("vault → entry");
    assert_eq!(by_vault.name(), "field-notes");
    assert_eq!(by_vault.canonical_path(), Some(canonical.as_path()));

    // The partiality, asserted rather than assumed: the vault axis is undefined
    // over a git-folder root, so INV-3 holds vacuously there.
    let git_folder = bound.by_name("archive").expect("the git-folder root binds");
    assert_eq!(git_folder.vault(), None);
    assert_eq!(bound.by_vault("archive"), None);

    // An unbound name answers None rather than guessing.
    assert_eq!(bound.by_name("assets"), None);
}

// ---------------------------------------------------------------------------
// The closed sets — complete and reachable
// ---------------------------------------------------------------------------

/// Every bind reason word is spelled once and is reachable.
#[test]
fn the_bind_reason_set_is_closed_and_every_word_is_reachable() {
    let words: Vec<&str> = MountReason::ALL.iter().map(|r| r.word()).collect();
    assert_eq!(
        words,
        [
            "mount-path-denied",
            "duplicate-mount-path",
            "nested-mount",
            "duplicate-vault-name",
            "declared-bound-mismatch",
        ],
        "the bind reason set is the mount-path law's, in its order"
    );

    // `a` declares its own name, so the first entry of every pair below binds
    // and the refusal under test is genuinely the second entry's (§8.4: the
    // first fault in file order is the only one reported).
    let (_keep, a) = root("a");
    let (_keep_b, b) = root("b");
    let (_keep_c, c) = root("c");
    let nested = a.join("inner");
    std::fs::create_dir_all(&nested).expect("nested");
    declare(&a, "a");
    declare(&b, "b");
    declare(&nested, "inner");
    declare(&c, "not-the-bound-name");

    let reached = [
        refuse(&config_of(&[vault_block("poison", &home())])).reason,
        refuse(&config_of(&[
            vault_block("a", &a),
            mount_block("a-again", &a, Some("a-again"), None),
        ]))
        .reason,
        refuse(&config_of(&[
            vault_block("a", &a),
            mount_block("inner", &nested, Some("inner"), None),
        ]))
        .reason,
        refuse(&config_of(&[
            mount_block("a", &a, Some("same"), None),
            mount_block("b", &b, Some("same"), None),
        ]))
        .reason,
        refuse(&config_of(&[vault_block("c", &c)])).reason,
    ];
    for reason in MountReason::ALL {
        assert!(
            reached.contains(&reason),
            "no input in this gate produces `{}` — an unreachable reason word",
            reason.word()
        );
    }
    assert_eq!(
        reached.len(),
        MountReason::ALL.len(),
        "one input per reason"
    );
}

/// The state vocabulary is closed: one `bound`, four `grey(...)`, one
/// `red(...)`, every word distinct — two states sharing a word would be
/// indistinguishable in the human line and in `--json` alike.
#[test]
fn the_state_vocabulary_is_closed_and_distinct() {
    let states = [
        MountState::Bound,
        MountState::PathUnseeable {
            detail: "no such file".to_string(),
        },
        MountState::Undeclared {
            declaration: PathBuf::from("/x/MERIDIAN.md"),
        },
        MountState::DeclarationUnreadable {
            declaration: PathBuf::from("/x/MERIDIAN.md"),
            detail: "no frontmatter".to_string(),
        },
        MountState::ClaimUnverifiable {
            detail: "codec".to_string(),
        },
        MountState::Drifted {
            pinned: "fp1.span2.b3.aa".to_string(),
            live: "fp1.span2.b3.bb".to_string(),
        },
    ];

    let words: Vec<&str> = states.iter().map(MountState::word).collect();
    assert_eq!(
        words,
        [
            "bound",
            "grey(path-unseeable)",
            "grey(undeclared)",
            "grey(declaration-unreadable)",
            "grey(claim-unverifiable)",
            "red(content-drifted)",
        ]
    );
    let mut unique = words.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), words.len(), "every state word is distinct");

    for state in &states {
        assert_eq!(
            state.refuses(),
            !matches!(state, MountState::Bound),
            "every state but `bound` refuses — grey and red alike (S3-R6)"
        );
        assert_eq!(
            state.detail().is_empty(),
            matches!(state, MountState::Bound),
            "every refusing state teaches; `bound` has nothing to teach"
        );
    }

    // None of the grey words collides with another plane's reason words.
    for word in &words {
        assert!(
            *word != "grey(cannot-assess)" && *word != "grey(unmounted)",
            "`{word}` collides with another plane's reason word"
        );
    }
}

/// A bind refusal renders in the same shape as a parse refusal: the config
/// path, the file line, the teaching middle, the no-partial-load clause, and a
/// `Fix:`.
#[test]
fn a_bind_refusal_renders_in_the_parse_refusal_shape() {
    let bind_refusal = refuse(&config_of(&[vault_block("poison", &home())])).to_string();
    let parse_refusal = crate::parse(
        "---\ntype: meridian-config\nversion: 1\n---\n\n```meridian-mount\nname: x\npath: /x\nprimary: nope\n```\n",
        Path::new("~/MERIDIAN.md"),
    )
    .expect_err("primary: nope refuses")
    .to_string();

    for text in [&bind_refusal, &parse_refusal] {
        assert!(text.starts_with("refused: ~/MERIDIAN.md line "), "{text}");
        assert!(text.contains(NO_PARTIAL_LOAD_CLAUSE), "{text}");
        assert!(text.contains(" Fix: "), "{text}");
        assert!(text.ends_with('.'), "{text}");
    }
}

// ---------------------------------------------------------------------------
// The reserved-name identity, and the state A / state D carry-through
// ---------------------------------------------------------------------------

/// The declaration filename is the config filename — one constant — and the
/// `type:` key tells the two uses apart: `MERIDIAN_CONFIG` aimed at a root's
/// declaration refuses loudly instead of half-loading.
#[test]
fn the_reserved_name_is_shared_and_the_type_key_separates_the_two_uses() {
    assert_eq!(DECLARATION_FILENAME, CONFIG_FILENAME);
    assert_ne!(DECLARATION_TYPE, crate::CONFIG_TYPE);

    let (_keep, dir) = root("field-notes");
    declare(&dir, "field-notes");
    let err = crate::resolve(&crate::Env {
        meridian_config: Some(dir.join(DECLARATION_FILENAME).display().to_string()),
        home: Some(dir.display().to_string()),
    })
    .expect_err("a root declaration is not a config");
    assert_eq!(err.reason, crate::Reason::WrongTypeValue);
    assert!(err.detail.contains(DECLARATION_TYPE), "{}", err.detail);
}

/// State A and state D reach the empty table through one call. Binding does
/// not reopen the nil-vs-empty distinction the config plane closed.
#[test]
fn absent_and_zero_mount_configs_bind_to_the_same_empty_table() {
    let absent = Resolution::Absent {
        path: PathBuf::from("/nowhere/MERIDIAN.md"),
    }
    .bind()
    .expect("state A binds");
    let zero = Resolution::Loaded(parse_config(
        "---\ntype: meridian-config\nversion: 1\n---\n\n# nothing declared\n",
    ))
    .bind()
    .expect("state D binds");

    assert_eq!(absent, zero, "state A and state D are ONE table");
    assert!(absent.mounts().is_empty());
    assert!(
        absent.is_clear(),
        "an empty table refuses nothing — it is a legitimate statement"
    );
}
