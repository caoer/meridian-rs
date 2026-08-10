//! The domain-excluded NOTE names a bounded sample; the COUNT is never bounded.
//!
//! This gate exists because the uncapped form was a real weapon, not a
//! hypothetical one. Measured 2026-08-10 on the newly registered mcp face: ONE
//! FAILED `walk` returned an error payload of 3,171,117 characters enumerating
//! 28,936 markdown files, because `voice_excluded` is emitted BEFORE the door
//! answers and a consumer folding stderr into its error string hands the whole
//! enumeration to a caller that is already retrying. An agent cannot un-read
//! that.
//!
//! The anti-silence law (session decision 0017, `docs/wire-contract.md` §12.1
//! enumerator clause) demands that an enumerator never exclude SILENTLY. It
//! does not demand unbounded prose: the COUNT, a sample, and a pointer to the
//! complete `excluded` key on the machine answer satisfy it — which is what
//! this file pins.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_home = tmp.path().join("xdg-cache");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    Sandbox {
        tmp,
        cache_home,
        home,
    }
}

impl Sandbox {
    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new(mrd_bin())
            .args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("spawn mrd")
    }
}

fn write(ws: &Path, rel: &str, body: &str) {
    let p = ws.join(rel);
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(p, body).expect("write");
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

/// How many excluded files the fixture creates. Well above the cap, so the
/// remainder clause has to do real arithmetic rather than sit at zero.
const EXCLUDED_FILES: usize = 12;
/// The cap the note is expected to honour — stated here independently of the
/// implementation constant ON PURPOSE: a test that imports the value it is
/// checking passes for any value.
const EXPECTED_SHOWN: usize = 3;

/// A workspace whose `meridian/domain.md` ignores `bulk/**`, so every file
/// under `bulk/` is real, on disk, and outside the hash domain.
fn workspace_with_excluded(sb: &Sandbox) -> PathBuf {
    let ws = sb.tmp.path().join("project");
    std::fs::create_dir_all(&ws).expect("mkdir");
    write(&ws, "a.md", "# A\n\nalpha links to nothing.\n");
    write(
        &ws,
        "meridian/domain.md",
        "---\nversion: 1\nignore:\n  - \"bulk/**\"\n---\n\nVendored copies do not move this \
         workspace's fingerprint.\n",
    );
    for i in 0..EXCLUDED_FILES {
        write(&ws, &format!("bulk/file{i:02}.md"), "# bulk\n\nexcluded.\n");
    }
    let init = sb.run(&ws, &["init"]);
    assert!(init.status.success(), "init: {}", stderr(&init));
    ws
}

#[test]
fn the_excluded_note_states_the_full_count_but_samples_the_paths() {
    let sb = sandbox();
    let ws = workspace_with_excluded(&sb);

    let out = sb.run(&ws, &["walk", "a.md", "--down"]);
    let said = stderr(&out);

    // Positive control FIRST: if the note never fired, everything below would
    // pass vacuously — a bound is trivially satisfied by an absent line.
    assert!(
        said.contains("outside the hash domain"),
        "the excluded note did not fire at all, so this gate would pass \
         vacuously — fixture is wrong, not the cap: {said}"
    );

    // The COUNT is the whole population and is never capped.
    assert!(
        said.contains(&format!("{EXCLUDED_FILES} markdown file(s)")),
        "the note must state the FULL count ({EXCLUDED_FILES}), which is the \
         half that makes exclusion non-silent: {said}"
    );

    // The SAMPLE is capped, and says so.
    let rest = EXCLUDED_FILES - EXPECTED_SHOWN;
    assert!(
        said.contains(&format!("and {rest} more")),
        "the note must say how many it did NOT name — a sample that does not \
         admit it is a sample reads as the whole list: {said}"
    );

    // The paths beyond the cap are absent. This is the assertion that actually
    // fails when the cap is removed.
    let named = (0..EXCLUDED_FILES)
        .filter(|i| said.contains(&format!("bulk/file{i:02}.md")))
        .count();
    assert_eq!(
        named, EXPECTED_SHOWN,
        "the note named {named} paths; the cap is {EXPECTED_SHOWN}. An \
         uncapped note is the 3.1-million-character payload this gate exists \
         to prevent: {said}"
    );

    // And the reader is told where the complete list lives, so capping the
    // prose does not lose the information — it moves it to the machine channel.
    assert!(
        said.contains("`excluded`"),
        "a capped note must point at the complete machine-readable list, or \
         the cap becomes the silence the enumerator clause forbids: {said}"
    );
}

/// The cap must not fire when there is nothing to cap: a population at or under
/// the cap names every member and claims no remainder.
#[test]
fn a_small_population_is_named_in_full_with_no_remainder_clause() {
    let sb = sandbox();
    let ws = sb.tmp.path().join("small");
    std::fs::create_dir_all(&ws).expect("mkdir");
    write(&ws, "a.md", "# A\n\nalpha.\n");
    write(
        &ws,
        "meridian/domain.md",
        "---\nversion: 1\nignore:\n  - \"bulk/**\"\n---\n\nignored.\n",
    );
    write(&ws, "bulk/only.md", "# one\n\nexcluded.\n");
    let init = sb.run(&ws, &["init"]);
    assert!(init.status.success(), "init: {}", stderr(&init));

    let said = stderr(&sb.run(&ws, &["walk", "a.md", "--down"]));
    assert!(
        said.contains("outside the hash domain"),
        "control: the note must fire here too: {said}"
    );
    assert!(
        said.contains("bulk/only.md"),
        "a population under the cap is named in full: {said}"
    );
    assert!(
        !said.contains(" more"),
        "there is no remainder, so the note must not claim one — a remainder \
         clause that fires at zero teaches readers to ignore it: {said}"
    );
}
