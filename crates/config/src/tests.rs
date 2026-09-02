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
path: /srv/vaults/field-notes
vault: field-notes
```
";

fn at(raw: &str) -> Result<Config, ConfigError> {
    parse(raw, Path::new("~/MERIDIAN.md"))
}

fn refuse(raw: &str) -> ConfigError {
    at(raw).expect_err("this input must refuse")
}

/// A minimal valid frontmatter with `body` after it — the shape every block
/// grammar test needs and none of them is about.
fn fm(body: &str) -> String {
    format!("---\ntype: meridian-config\nversion: 1\n---\n\n{body}")
}

/// The kind sweep: `kind:` left the mount schema — vault-ness
/// is carried by `vault:` presence alone, and the primary designation is
/// legal on any mount. A config still carrying the field refuses through the
/// unknown-field door, naming the line and the removal — no silent tolerance,
/// no compatibility window, exactly as `primary:` refused on an engine too
/// old to know it. The deploy order this creates is not optional: install the
/// engine, remove the `kind:` lines from ~/MERIDIAN.md, then switch anything
/// that dials it.
#[test]
fn a_stale_kind_line_refuses_loud_with_the_removal_remedy() {
    let raw = "\
---
type: meridian-config
version: 1
---

```meridian-mount
name: field-notes
path: /w
kind: vault
vault: field-notes
```
";
    let err = refuse(raw);
    assert_eq!(err.reason, Reason::UnknownField);
    let text = err.to_string();
    assert!(text.contains("unknown field `kind`"), "{text}");
    assert!(text.contains("remove the line"), "{text}");
}

/// `alias:` is the sixth field (§5.1b), and it parses as a lookup spelling —
/// the value survives to [`MountEntry::alias`] and nothing else about the block
/// changes. The typo door stays shut by the closed schema: `alais:` is an
/// ordinary `unknown-field`, so the silent-typo hazard §4 names is closed here
/// exactly as it is for every other optional field.
#[test]
fn alias_parses_as_the_sixth_field_and_a_typo_refuses() {
    let raw = "\
---
type: meridian-config
version: 1
---

```meridian-mount
name: field-notes-sessions
path: /srv/vaults/field-notes-sessions
vault: field-notes-sessions
alias: sessions
```
";
    let config = at(raw).expect("a well-formed alias parses");
    let mount = &config.mounts()[0];
    assert_eq!(mount.name, "field-notes-sessions");
    assert_eq!(mount.alias.as_deref(), Some("sessions"));

    let typo = refuse(&raw.replace("alias: sessions", "alais: sessions"));
    assert_eq!(typo.reason, Reason::UnknownField);
    assert!(typo.to_string().contains("unknown field `alais`"), "{typo}");
    // The legal set the refusal enumerates must name the field that exists,
    // or the remedy sends a reader looking for a field the engine denies.
    assert!(
        typo.to_string()
            .contains("name, path, primary, vault, pin, alias"),
        "{typo}"
    );
}

/// Canonical order is the table's (§5.1) and `alias:` is LAST — a block that
/// writes it before `pin:` refuses `field-out-of-order` like any other
/// transposition. Pinned because the field was appended: an implementation that
/// slotted it anywhere but the end would pass every other alias test.
#[test]
fn alias_is_last_in_canonical_order() {
    let err = refuse(&fm(
        "```meridian-mount\nname: a\npath: /x\nalias: b\npin: fp1.span2.b3.ab\n```\n",
    ));
    assert_eq!(err.reason, Reason::FieldOutOfOrder);
    assert!(
        err.to_string()
            .contains("name, path, primary, vault, pin, alias"),
        "{err}"
    );
}

/// An empty `alias:` refuses rather than becoming a second spelling for "no
/// alias" — the rule `primary:` and `vault:` already hold. A value outside the
/// §5.2 charset refuses too: an alias occupies the same `root:` position as a
/// name, so a spelling no address can carry could never be used.
#[test]
fn an_empty_or_uncanonical_alias_refuses() {
    let empty = refuse(&fm("```meridian-mount\nname: a\npath: /x\nalias:\n```\n"));
    assert_eq!(empty.reason, Reason::MalformedLine, "{empty}");

    let blank = refuse(&fm("```meridian-mount\nname: a\npath: /x\nalias: \n```\n"));
    assert_eq!(blank.reason, Reason::BadValue, "{blank}");
    assert!(blank.to_string().contains("`alias:` is empty"), "{blank}");

    let shouty = refuse(&fm(
        "```meridian-mount\nname: a\npath: /x\nalias: Sessions\n```\n",
    ));
    assert_eq!(shouty.reason, Reason::BadValue, "{shouty}");
    assert!(shouty.to_string().contains("a mount alias"), "{shouty}");
}

/// An alias equal to any mount's `name` refuses the WHOLE table — including a
/// name declared LATER in the file, which is the half an incremental check
/// misses, and including the alias's own mount, where the line is unreachable
/// rather than ambiguous.
#[test]
fn an_alias_shadowing_a_name_refuses_the_whole_table() {
    // The shadowed name is declared AFTER the alias that shadows it.
    let later = refuse(&fm(
        "```meridian-mount\nname: a\npath: /x\nalias: b\n```\n\n```meridian-mount\nname: b\npath: /y\n```\n",
    ));
    assert_eq!(later.reason, Reason::AliasShadowsName);
    assert!(
        later.to_string().contains(NO_PARTIAL_LOAD_CLAUSE),
        "{later}"
    );
    assert!(
        later.to_string().contains("looked up by name first"),
        "the refusal must teach WHY a shadow is fatal: {later}"
    );

    // Its own name: legal-looking, does nothing, refuses.
    let own = refuse(&fm("```meridian-mount\nname: a\npath: /x\nalias: a\n```\n"));
    assert_eq!(own.reason, Reason::AliasShadowsName);

    // Two mounts claiming one alias: a key with two values.
    let twice = refuse(&fm(
        "```meridian-mount\nname: a\npath: /x\nalias: c\n```\n\n```meridian-mount\nname: b\npath: /y\nalias: c\n```\n",
    ));
    assert_eq!(twice.reason, Reason::AliasShadowsName);
    assert!(
        twice.to_string().contains("already declared at line"),
        "{twice}"
    );
}

/// The exemplar is produced by a real parse, not merely contained in one: a
/// drift in any of §8.3's three mandatory clauses is a byte-level failure.
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
paths: /srv/vaults/field-notes-sessions
path: /srv/vaults/field-notes
vault: field-notes
```
";
    // The exemplar's line 14 is where `paths:` sits — asserted, so the fixture
    // and the pinned string cannot drift apart silently.
    assert_eq!(
        raw.lines().nth(13),
        Some("paths: /srv/vaults/field-notes-sessions")
    );

    let err = refuse(raw);
    assert_eq!(err.reason, Reason::UnknownField);
    assert_eq!(err.line, Some(14));
    assert_eq!(err.to_string(), UNKNOWN_FIELD_REFUSAL_EXEMPLAR);
}

/// Every refusal carries §8.3's three mandatory clauses, whatever the reason.
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

/// A CLOSED but EMPTY frontmatter block refuses on the missing key, not on the
/// fence.
///
/// Schema §4's malformed-frontmatter table gives `no-frontmatter` exactly two
/// conditions — the file does not open with `---\n`, or the fence never closes.
/// An empty closed block is neither: it opens and it closes. What it does is
/// declare no `type:`, which §4's key table sends to `missing-required-key`.
/// The markdown parser mints no frontmatter node for `---\n---`, so before this
/// gate the door refused it as `no-frontmatter` and told the author their file
/// "does not open with a closed `---` frontmatter block" — a false statement
/// about bytes they are looking at, on the one door whose whole job is to
/// teach.
#[test]
fn a_closed_empty_frontmatter_refuses_on_the_missing_key_not_the_fence() {
    for raw in [
        "---\n---\n\n# a mount table\n",
        "---\n\n---\n\n# a mount table\n",
        "---\n   \n---\n\n# a mount table\n",
    ] {
        let err = refuse(raw);
        assert_eq!(
            err.reason,
            Reason::MissingRequiredKey,
            "an empty closed frontmatter declares no `type:`: {err}"
        );
        assert_eq!(err.line, Some(1), "§8.1a puts a frontmatter key on line 1");
        let text = err.to_string();
        assert!(
            text.contains("`type:`"),
            "the refusal must name the missing key: {text}"
        );
        assert!(
            !text.contains("does not open with a closed"),
            "the file DOES open with a closed fence — the refusal may not say otherwise: {text}"
        );
    }
}

/// The two conditions schema §4 does give `no-frontmatter` still reach it.
#[test]
fn no_frontmatter_still_covers_its_own_two_conditions() {
    // Does not open with `---\n`.
    assert_eq!(refuse("# no frontmatter\n").reason, Reason::NoFrontmatter);
    // Opens, but the fence never closes.
    assert_eq!(
        refuse("---\ntype: meridian-config\nversion: 1\n\n# unterminated\n").reason,
        Reason::NoFrontmatter
    );
    // A fence that opens and never closes with nothing but blank lines after
    // it is still an unclosed fence, not an empty block.
    assert_eq!(refuse("---\n\n\n").reason, Reason::NoFrontmatter);
}

/// The closed reason set is exactly schema §8.2's, and every word is
/// reachable.
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
            "bad-value",
            "malformed-line",
            "unterminated-block",
            "duplicate-mount-name",
            "duplicate-tool-name",
            "duplicate-primary-designation",
            "alias-shadows-name",
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
        (Reason::MissingRequiredField, refuse(&mount("path: /x\n"))),
        (Reason::UnknownField, refuse(&mount("name: a\npaths: /x\n"))),
        (
            Reason::DuplicateField,
            refuse(&mount("name: a\npath: /x\npath: /y\n")),
        ),
        (
            Reason::FieldOutOfOrder,
            refuse(&mount("path: /x\nname: a\n")),
        ),
        (
            Reason::BadValue,
            refuse(&mount("name: a\npath: /x\nprimary: maybe\n")),
        ),
        (
            Reason::MalformedLine,
            refuse(&mount("name: a\n\npath: /x\n")),
        ),
        (
            Reason::UnterminatedBlock,
            refuse(&fm("```meridian-mount\nname: a\npath: /x\n")),
        ),
        (
            Reason::DuplicateMountName,
            refuse(&fm(
                "```meridian-mount\nname: a\npath: /x\n```\n\n```meridian-mount\nname: a\npath: /y\n```\n",
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
        (
            Reason::DuplicatePrimaryDesignation,
            refuse(&fm(
                "```meridian-mount\nname: a\npath: /x\nprimary: true\nvault: a\n```\n\n```meridian-mount\nname: b\npath: /y\nprimary: true\nvault: b\n```\n",
            )),
        ),
        (
            Reason::AliasShadowsName,
            refuse(&fm(
                "```meridian-mount\nname: a\npath: /x\n```\n\n```meridian-mount\nname: b\npath: /y\nalias: a\n```\n",
            )),
        ),
    ]
}

/// State D is a behavioural identity with state A: the two reach the same
/// mount table through the same accessor, and the config's own rev is the
/// single permitted difference. Gates the nil-vs-empty bug.
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
/// path.
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

/// `config_rev` is the shipped `file_rev` law, checked against an independent
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

/// An out-of-band edit moves the config's rev and fingerprint — asserted as
/// state changes, never an exit status. `~/MERIDIAN.md` lives in `$HOME`, a
/// denied workspace path with no attestation baseline, so the rev is a
/// reported number, not a drift verdict.
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
        SINGLE.replace("/srv/vaults", "/Volumes/wiki"),
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
/// legal name can collide with an address operator.
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

/// Prose is prose, and the machine surface is located through the model tree.
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
path: /srv/vaults/field-notes
vault: field-notes
```
";
    let config = at(raw).expect("the decoys are prose, so this parses");
    assert_eq!(config.mounts().len(), 1, "exactly one mount, not four");
    assert_eq!(config.mounts()[0].name, "field-notes");
}

/// A refused config leaves no partially-populated state observable — asserted
/// as the absence, not just as the error. Each input carries a valid mount
/// block, so a build that half-loads looks healthy.
#[test]
fn a_refused_config_publishes_nothing() {
    let valid_block = "```meridian-mount\nname: field-notes\npath: /srv/vaults/field-notes\nvault: field-notes\n```\n";
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
        // `Config`'s fields are private and `parse` is its only constructor,
        // so a partially-populated table is not a state this API can be in.
        assert!(result.ok().is_none(), "{label} publishes no mount table");
    }
}

/// The pin survives parse verbatim: normalizing, truncating, or re-minting
/// the token breaks the claim it carries.
#[test]
fn a_pin_is_carried_verbatim_and_codec_agnostically() {
    let span = "fp1.span2.b3.40b167ed9b42a2beadb7c441b214efdc93069ef443a1cc2b5ae2ccda4cf03152";
    // A plain-folder mount may pin a different codec.
    let file_codec = format!("fp1.raw.b3.{}", "ab".repeat(32));
    let raw = format!(
        "---\ntype: meridian-config\nversion: 1\n---\n\n```meridian-mount\nname: a\npath: /x\nvault: a\npin: {span}\n```\n\n```meridian-mount\nname: b\npath: /y\npin: {file_codec}\n```\n"
    );
    let config = at(&raw).expect("both codecs are well-formed tokens");
    assert_eq!(config.mounts()[0].pin.as_deref(), Some(span));
    assert_eq!(config.mounts()[1].pin.as_deref(), Some(file_codec.as_str()));

    // A token whose hash-fn this build does not know still parses; whether it
    // verifies is `verify_content`'s question, never parse's.
    let future = "fp2.tree.sha256.00ff";
    let raw = format!(
        "---\ntype: meridian-config\nversion: 1\n---\n\n```meridian-mount\nname: a\npath: /x\npin: {future}\n```\n"
    );
    assert_eq!(
        at(&raw).expect("a future codec is well-formed").mounts()[0]
            .pin
            .as_deref(),
        Some(future)
    );
}
