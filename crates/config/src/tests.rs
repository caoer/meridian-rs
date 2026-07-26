//! The config plane's own gates. The 37-case fixture corpus is driven from
//! `crates/testsuite/tests/meridian_md.rs` — that binary owns the pack; these
//! own the properties a fixture cannot express: the refusal exemplar produced
//! verbatim, the closed reason set's reachability, the state-A/state-D
//! identity, and the self-hosting rev with the limit it cannot cross.

use super::*;

/// A well-formed minimal config, for tests that need one.
const SINGLE: &str = "\
---
type: meridian-config
version: 1
---

# My system

```meridian-mount
name: field-notes
path: /Users/Shared/projects/field-notes
kind: vault
vault: field-notes
```
";

fn at(raw: &str) -> Result<Config, ConfigError> {
    parse(raw, Path::new("~/MERIDIAN.md"))
}

fn refuse(raw: &str) -> ConfigError {
    at(raw).expect_err("this input must refuse")
}

/// The house pinned-`const` exemplar pattern, in its strongest form: the
/// exemplar is **produced by a real parse**, not merely contained in one. A
/// drift in any of §8.3's three mandatory clauses — the line, the
/// no-partial-load sentence, the `Fix:` — is a byte-level failure here.
#[test]
fn refusal_exemplar_is_produced_not_asserted() {
    let raw = "\
---
type: meridian-config
version: 1
---

# My system

The mount below carries a field the engine does not read.

## Roots

```meridian-mount
name: field-notes
paths: /Users/Shared/projects/field-notes-sessions
path: /Users/Shared/projects/field-notes
kind: vault
vault: field-notes
```
";
    // The exemplar's line 14 is where `paths:` sits — asserted, so the fixture
    // and the pinned string cannot drift apart silently.
    assert_eq!(
        raw.lines().nth(13),
        Some("paths: /Users/Shared/projects/field-notes-sessions")
    );

    let err = refuse(raw);
    assert_eq!(err.reason, Reason::UnknownField);
    assert_eq!(err.line, Some(14));
    assert_eq!(err.to_string(), UNKNOWN_FIELD_REFUSAL_EXEMPLAR);
}

/// Every refusal carries §8.3's three mandatory clauses, whatever the reason.
/// A guard that only the exemplar's own reason satisfies is a guard on one
/// path.
#[test]
fn every_refusal_teaches_line_no_partial_load_and_a_fix() {
    for (reason, err) in one_of_each_reason() {
        let text = err.to_string();
        assert!(text.starts_with("refused: "), "{}: {text}", reason.word());
        assert!(
            text.contains(NO_PARTIAL_LOAD_CLAUSE),
            "{} must state that nothing loaded: {text}",
            reason.word()
        );
        assert!(
            text.contains(" Fix: "),
            "{} must teach the fix: {text}",
            reason.word()
        );
        // Only the two reasons with no bytes to point at may omit the line.
        let lineless = matches!(
            reason,
            Reason::ConfigPathUnusable | Reason::HomeUnresolvable
        );
        assert_eq!(
            err.line.is_none(),
            lineless,
            "{} line-carrying is §8.1a's, not a choice: {text}",
            reason.word()
        );
    }
}

/// The closed reason set is exactly schema §8.2's, and **every word is
/// reachable**. A reason word no code path can emit is a spelling, not a
/// guard; this is the anti-vacuity half of the closed set.
#[test]
fn the_closed_reason_set_is_complete_and_reachable() {
    let words: Vec<&str> = Reason::ALL.iter().map(|r| r.word()).collect();
    assert_eq!(
        words,
        [
            "config-path-unusable",
            "home-unresolvable",
            "no-frontmatter",
            "frontmatter-unparseable",
            "missing-required-key",
            "wrong-type-value",
            "unsupported-version",
            "missing-required-field",
            "unknown-field",
            "duplicate-field",
            "field-out-of-order",
            "field-not-permitted-for-kind",
            "bad-value",
            "malformed-line",
            "unterminated-block",
            "duplicate-mount-name",
            "duplicate-tool-name",
        ],
        "the reason set is schema §8.2's table, in its order"
    );

    let reached: Vec<Reason> = one_of_each_reason().into_iter().map(|(r, _)| r).collect();
    for reason in Reason::ALL {
        assert!(
            reached.contains(&reason),
            "no input in this gate produces `{}` — an unreachable reason word",
            reason.word()
        );
    }
    assert_eq!(reached.len(), Reason::ALL.len(), "one input per reason");
}

/// One input per reason word, each producing exactly that word. Kept in §8.2's
/// order so a reader can walk the table against it.
fn one_of_each_reason() -> Vec<(Reason, ConfigError)> {
    let home = tempfile::tempdir().expect("tempdir");
    let missing = home.path().join("nowhere").join("MERIDIAN.md");
    let env = Env {
        meridian_config: Some(missing.display().to_string()),
        home: Some(home.path().display().to_string()),
    };
    let unusable = resolve(&env).expect_err("a stated path that is not there refuses");

    let no_home = resolve(&Env {
        meridian_config: None,
        home: None,
    })
    .expect_err("an unbuildable rung 2 refuses");

    let fm = |body: &str| format!("---\ntype: meridian-config\nversion: 1\n---\n\n{body}");
    let mount = |body: &str| fm(&format!("```meridian-mount\n{body}```\n"));
    let tool = |body: &str| fm(&format!("```meridian-tool\n{body}```\n"));

    vec![
        (Reason::ConfigPathUnusable, unusable),
        (Reason::HomeUnresolvable, no_home),
        (Reason::NoFrontmatter, refuse("# no frontmatter\n")),
        (
            Reason::FrontmatterUnparseable,
            refuse("---\ntype: meridian-config\nversion: 1\ntags: [a, b\n---\n"),
        ),
        (
            Reason::MissingRequiredKey,
            refuse("---\nversion: 1\n---\n\n# no type\n"),
        ),
        (
            Reason::WrongTypeValue,
            refuse("---\ntype: note\nversion: 1\n---\n\n# a page\n"),
        ),
        (
            Reason::UnsupportedVersion,
            refuse("---\ntype: meridian-config\nversion: 2\n---\n\n# from the future\n"),
        ),
        (
            Reason::MissingRequiredField,
            refuse(&mount("path: /x\nkind: git-folder\n")),
        ),
        (Reason::UnknownField, refuse(&mount("name: a\npaths: /x\n"))),
        (
            Reason::DuplicateField,
            refuse(&mount("name: a\npath: /x\npath: /y\n")),
        ),
        (
            Reason::FieldOutOfOrder,
            refuse(&mount("name: a\nkind: git-folder\npath: /x\n")),
        ),
        (
            Reason::FieldNotPermittedForKind,
            refuse(&mount("name: a\npath: /x\nkind: git-folder\nvault: a\n")),
        ),
        (
            Reason::BadValue,
            refuse(&mount("name: a\npath: /x\nkind: obsidian\n")),
        ),
        (
            Reason::MalformedLine,
            refuse(&mount("name: a\n\npath: /x\n")),
        ),
        (
            Reason::UnterminatedBlock,
            refuse(&fm(
                "```meridian-mount\nname: a\npath: /x\nkind: git-folder\n",
            )),
        ),
        (
            Reason::DuplicateMountName,
            refuse(&fm(
                "```meridian-mount\nname: a\npath: /x\nkind: git-folder\n```\n\n```meridian-mount\nname: a\npath: /y\nkind: git-folder\n```\n",
            )),
        ),
        (
            Reason::DuplicateToolName,
            refuse(&format!(
                "{}\n{}",
                tool("name: t\nkind: skill\n").trim_end(),
                "```meridian-tool\nname: t\nkind: mcp\n```\n"
            )),
        ),
    ]
}

/// State D is a behavioural IDENTITY with state A, not a similarity: the two
/// reach the same mount table through the same accessor. The config's own rev
/// is the single permitted difference, and it is an observation, never a
/// branch.
///
/// This is the gate against the nil-vs-empty bug: an implementation that gave
/// state D its own variant, or its own empty-table constant, passes every
/// other case in the pack and fails here.
#[test]
fn state_d_and_state_a_reach_one_mount_table() {
    let home = tempfile::tempdir().expect("tempdir");
    let env = Env {
        meridian_config: None,
        home: Some(home.path().display().to_string()),
    };

    let absent = resolve(&env).expect("an absent config is not an error");
    assert!(matches!(absent, Resolution::Absent { .. }));

    std::fs::write(
        home.path().join(CONFIG_FILENAME),
        "---\ntype: meridian-config\nversion: 1\n---\n\n# My system\n\nI have not declared any roots yet.\n",
    )
    .expect("write");
    let zero = resolve(&env).expect("a zero-mount config is not an error");

    assert_eq!(absent.mounts(), zero.mounts(), "one mount table, not two");
    assert!(absent.mounts().is_empty() && zero.mounts().is_empty());
    assert_eq!(absent.tools(), zero.tools());
    assert_eq!(absent.path(), zero.path(), "the same resolved path");

    assert_eq!(absent.file_rev(), None, "state A has no file to rev");
    assert!(
        zero.file_rev().is_some(),
        "state D parsed a file, so it carries that file's rev — the ONE permitted difference"
    );
}

/// The chain is exactly two rungs and the empty/whitespace override states no
/// path — the same nil-vs-empty distinction state D draws, on the env axis.
#[test]
fn the_chain_has_two_rungs_and_an_empty_override_states_nothing() {
    let home = tempfile::tempdir().expect("tempdir");
    let default = home.path().join(CONFIG_FILENAME);
    std::fs::write(&default, SINGLE).expect("write");

    let elsewhere = home.path().join("elsewhere.md");
    std::fs::write(
        &elsewhere,
        SINGLE.replace("name: field-notes", "name: sessions"),
    )
    .expect("write");

    let base = Env {
        meridian_config: None,
        home: Some(home.path().display().to_string()),
    };
    for stated in [None, Some(String::new()), Some("   ".to_string())] {
        let env = Env {
            meridian_config: stated.clone(),
            home: base.home.clone(),
        };
        assert_eq!(
            resolve(&env).expect("rung 2").mounts()[0].name,
            "field-notes",
            "MERIDIAN_CONFIG={stated:?} states no path, so the chain falls to rung 2"
        );
    }

    let env = Env {
        meridian_config: Some(elsewhere.display().to_string()),
        home: base.home.clone(),
    };
    assert_eq!(
        resolve(&env).expect("rung 1").mounts()[0].name,
        "sessions",
        "a stated path wins over the default"
    );
}

/// `config_rev` is the SHIPPED `file_rev` law, checked against an independent
/// oracle rather than against the code that computes it.
#[test]
fn the_config_rev_is_the_shipped_file_rev_law() {
    let config = at(SINGLE).expect("parses");
    let oracle = model::build(SINGLE.to_string(), syntax::parse(SINGLE))
        .root
        .node_rev
        .0;
    assert_eq!(config.file_rev(), oracle, "no new rev law is minted here");
    assert_eq!(config.file_rev().len(), 16);
    assert!(config.file_rev().chars().all(|c| c.is_ascii_hexdigit()));
    assert!(
        config
            .fingerprint()
            .and_then(model::fingerprint::parse_fingerprint)
            .is_some(),
        "the config carries a fingerprint like any page"
    );
}

/// Criterion 1's last clause, and the limit S3-R7 says it cannot cross —
/// asserted in ONE test so the claim and its bound cannot drift apart.
///
/// **What holds:** an out-of-band edit moves the config's rev AND its
/// fingerprint. Both are asserted as STATE CHANGES (R40), never as an exit
/// status.
///
/// **What does not:** `~/MERIDIAN.md` lives in `$HOME`, which is a DENIED
/// workspace path — it has no git repo, no receipt journal and no merkle hash
/// domain, and it can never be promoted into one to acquire them. So the rev
/// is a reported number that moves; it is **not** a drift verdict, because
/// there is no attestation baseline for it to be a verdict against. Claiming
/// drift detection on this surface is the false claim S3-R7 struck from the
/// plan.
#[test]
fn an_out_of_band_edit_moves_the_rev_and_the_home_limit_still_holds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join(CONFIG_FILENAME);
    std::fs::write(&file, SINGLE).expect("write");
    let env = Env {
        meridian_config: Some(file.display().to_string()),
        home: Some(dir.path().display().to_string()),
    };

    let before = resolve(&env).expect("loads");
    let before_rev = before
        .file_rev()
        .expect("a parsed config carries a rev")
        .to_string();
    let before_fp = before
        .config()
        .and_then(Config::fingerprint)
        .expect("and a fingerprint")
        .to_string();

    // The out-of-band edit: a human changes the file behind the engine's back.
    std::fs::write(
        &file,
        SINGLE.replace("/Users/Shared/projects", "/Volumes/wiki"),
    )
    .expect("edit");

    let after = resolve(&env).expect("still loads");
    let after_rev = after.file_rev().expect("rev").to_string();
    let after_fp = after
        .config()
        .and_then(Config::fingerprint)
        .expect("fingerprint")
        .to_string();

    assert_ne!(before_rev, after_rev, "the edit MOVED the config's rev");
    assert_ne!(before_fp, after_fp, "and its fingerprint");
    assert_eq!(
        after.mounts()[0].path,
        "/Volumes/wiki/field-notes",
        "and the parsed value the rev covers"
    );

    // The stated limit, measured rather than described.
    let home =
        std::env::var("HOME").expect("this gate states a limit about $HOME, so it needs one");
    assert_eq!(
        workspace::deny_reason(Path::new(&home)),
        Some(workspace::DenyReason::HomeDir),
        "the default config's own directory is a DENIED workspace path, so its rev can never become an attested one"
    );
}

/// The charset is the complement of the address grammar's operator set, so no
/// legal name can collide with an address operator — and the acceptance half
/// is asserted in the same breath (S3-R8(c)).
#[test]
fn the_root_name_charset_is_the_address_operators_complement() {
    for legal in ["a", "field-notes", "sessions", "wiki2", "0", "a-b-c-9"] {
        assert!(
            check_name(legal, Path::new("x"), 1, "a canonical root name").is_ok(),
            "`{legal}` is a legal canonical root name"
        );
    }
    for (illegal, why) in [
        ("", "empty"),
        ("home_wiki", "underscore (CHARSET-GUARD)"),
        ("Home-Wiki", "uppercase"),
        ("home wiki", "whitespace"),
        ("home:wiki", "the root separator"),
        ("home/wiki", "the path separator"),
        ("home.wiki", "the fingerprint-token separator"),
        ("home#wiki", "the selector separator"),
        ("home@wiki", "the fp decoration"),
        ("-wiki", "leading dash"),
        ("wiki-", "trailing dash"),
    ] {
        let err = check_name(illegal, Path::new("x"), 7, "a canonical root name").expect_err(why);
        assert_eq!(err.reason, Reason::BadValue, "{why}");
        assert_eq!(err.line, Some(7));
        assert!(
            err.to_string().contains(NAME_CHARSET),
            "{why}: teaches the charset"
        );
    }
    assert!(check_name(&"a".repeat(64), Path::new("x"), 1, "n").is_ok());
    assert!(check_name(&"a".repeat(65), Path::new("x"), 1, "n").is_err());
}

/// Prose is prose, and the machine surface is located through the MODEL TREE.
/// A reader that scans lines for `name:`/`path:` loads the decoys.
#[test]
fn decoys_are_inert_and_the_real_block_is_not() {
    let raw = "\
---
type: meridian-config
version: 1
---

# Decoys

```yaml
name: not-a-mount
path: /tmp/no
kind: vault
vault: not-a-mount
```

````text
```meridian-mount
name: also-not
path: /tmp/no
kind: git-folder
```
````

    name: indented-not
    path: /tmp/no
    kind: git-folder

Writing `meridian-mount` inline opens nothing, and neither does saying
kind: git-folder in a sentence.

```meridian-mount
name: field-notes
path: /Users/Shared/projects/field-notes
kind: vault
vault: field-notes
```
";
    let config = at(raw).expect("the decoys are prose, so this parses");
    assert_eq!(config.mounts().len(), 1, "exactly one mount, not four");
    assert_eq!(config.mounts()[0].name, "field-notes");
}

/// A refused config leaves NO partially-populated state observable to any
/// caller — asserted as the ABSENCE, not just as the error. The inputs each
/// carry a perfectly valid mount block, so a build that half-loads looks
/// healthy and would pass a test that only checked `is_err()` on an empty file.
#[test]
fn a_refused_config_publishes_nothing() {
    let valid_block = "```meridian-mount\nname: field-notes\npath: /Users/Shared/projects/field-notes\nkind: vault\nvault: field-notes\n```\n";
    for (label, raw) in [
        (
            "no frontmatter",
            format!("# no frontmatter\n\n{valid_block}"),
        ),
        (
            "unsupported version",
            format!("---\ntype: meridian-config\nversion: 2\n---\n\n{valid_block}"),
        ),
        (
            "a later malformed block",
            format!(
                "---\ntype: meridian-config\nversion: 1\n---\n\n{valid_block}\n```meridian-mount\nname: broken\n\n```\n"
            ),
        ),
    ] {
        let result = at(&raw);
        assert!(result.is_err(), "{label} must refuse");
        // There is no Config value at all — `Config`'s fields are private and
        // `parse` is its only constructor, so "a partially-populated table" is
        // not a state this API can be in. The type carries the guarantee; this
        // asserts the type's reachability, and the fixture pack's `expect_not`
        // cases assert it per malformed class.
        assert!(result.ok().is_none(), "{label} publishes no mount table");
    }
}

/// The pin survives parse VERBATIM. A reader that normalizes, truncates to the
/// `@fp` display form, or re-mints the token breaks the claim it exists to
/// carry — and mount-as-claim is the SOLE mechanism by which the mount table's
/// own integrity is checkable (S3-R7).
#[test]
fn a_pin_is_carried_verbatim_and_codec_agnostically() {
    let span = "fp1.span2.b3.40b167ed9b42a2beadb7c441b214efdc93069ef443a1cc2b5ae2ccda4cf03152";
    // A git-folder root's pin grain is the FILE, so its codec differs. A reader
    // that checks for the literal `fp1.span2.b3.` prefix refuses this.
    let file_codec = format!("fp1.raw.b3.{}", "ab".repeat(32));
    let raw = format!(
        "---\ntype: meridian-config\nversion: 1\n---\n\n```meridian-mount\nname: a\npath: /x\nkind: vault\nvault: a\npin: {span}\n```\n\n```meridian-mount\nname: b\npath: /y\nkind: git-folder\npin: {file_codec}\n```\n"
    );
    let config = at(&raw).expect("both codecs are well-formed tokens");
    assert_eq!(config.mounts()[0].pin.as_deref(), Some(span));
    assert_eq!(config.mounts()[1].pin.as_deref(), Some(file_codec.as_str()));

    // A token whose hash-fn this build does not know still PARSES: whether it
    // can be verified is `verify_content`'s question, never parse's. Refusing
    // it here would forbid a future codec at the config door.
    let future = "fp2.tree.sha256.00ff";
    let raw = format!(
        "---\ntype: meridian-config\nversion: 1\n---\n\n```meridian-mount\nname: a\npath: /x\nkind: git-folder\npin: {future}\n```\n"
    );
    assert_eq!(
        at(&raw).expect("a future codec is well-formed").mounts()[0]
            .pin
            .as_deref(),
        Some(future)
    );
}
