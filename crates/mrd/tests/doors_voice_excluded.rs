//! The certifying doors voice the domain-excluded population.
//!
//! `mrd check` and the `mrd repair` sweep answer a POPULATION the caller did
//! not name: every pin in the corpus. Both owe the enumerator clause (session
//! decision 0017, `docs/wire-contract.md` §12.1): an enumerator MAY exclude
//! what its attestation cannot reach — an out-of-domain page's bytes cannot
//! move the fingerprint the answer is stamped with — but it may never exclude
//! SILENTLY. Before this gate, both doors voiced only the unserved class and
//! a pin living in an excluded page vanished from the assessment without a
//! word.
//!
//! `mrd repair <PAGE>` is a DOOR, not an enumeration (§12.1 door clause): the
//! named page is served (`admit_named_page`) and no census is voiced — a door
//! that voiced one would name its own subject as an exclusion.
//!
//! `mrd retire` is deliberately NOT covered here: it publishes the same
//! population as `files_excluded` inside its own report, both faces, which is
//! the stronger form — the census is part of the answer, not a note beside it.

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
    write(&ws, "bulk/vendored.md", "# bulk\n\nexcluded.\n");
    let init = sb.run(&ws, &["init"]);
    assert!(init.status.success(), "init: {}", stderr(&init));
    ws
}

#[test]
fn check_voices_the_domain_excluded_population() {
    let sb = sandbox();
    let ws = workspace_with_excluded(&sb);

    let out = sb.run(&ws, &["check"]);
    let said = stderr(&out);
    assert!(
        said.contains("outside the hash domain") && said.contains("bulk/vendored.md"),
        "`mrd check` assesses every pin the corpus carries — a population the \
         caller did not name — so it owes the census of what that assessment \
         cannot see. Silence here is the enumerator-clause defect: {said}"
    );
}

#[test]
fn repair_sweep_voices_the_domain_excluded_population() {
    let sb = sandbox();
    let ws = workspace_with_excluded(&sb);

    let out = sb.run(&ws, &["repair", "--dry"]);
    let said = stderr(&out);
    assert!(
        said.contains("outside the hash domain") && said.contains("bulk/vendored.md"),
        "a pageless `mrd repair` sweeps every lock in the corpus — an \
         enumeration — so a lock living in an excluded page must not vanish \
         silently from the sweep: {said}"
    );
}

#[test]
fn repair_named_page_is_a_door_and_stays_quiet() {
    let sb = sandbox();
    let ws = workspace_with_excluded(&sb);

    let out = sb.run(&ws, &["repair", "a.md", "--dry"]);
    let said = stderr(&out);
    assert!(
        !said.contains("outside the hash domain"),
        "`mrd repair <PAGE>` is a door: the named page is served, and a door \
         that voices a census names its own subject as an exclusion: {said}"
    );
}
