//! D-G — `mrd repair` crosses into MOUNTED roots (cross-root pin design,
//! session 12-04-f2-mrd-integration): a form-3 pin whose root is bound is
//! assessed in place — target read from that root's checkout, history walked
//! in that root's repo, recovered `hash` written into the HOME lock. An
//! unmounted root's rows stay skip-and-state. The floor and the forgery
//! invariant hold across the crossing: only `hash` moves.
//!
//! One env-dependent test fn by design (`MERIDIAN_CONFIG` is process-global;
//! the `walk_op.rs` precedent).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
}

/// The pinned page at mint time — the section this test attests sits BETWEEN
/// two others, so its span is bounded identically in every version (the
/// EOF-boundary near-miss is a separate, known finding — fa8f06d0's run1).
const DOC_V1: &str = "# Doc\n\n## Design\n\nthe real design note.\n\n## Tail\n\nv1 tail.\n";
/// A later version: the SECTION bytes are intact, the tail changed — a
/// different whole-file blob that still verifies the pin.
const DOC_V2: &str = "# Doc\n\n## Design\n\nthe real design note.\n\n## Tail\n\nlater tail.\n";
/// The drift: the pinned section itself rewritten.
const DOC_DRIFTED: &str = "# Doc\n\n## Design\n\nDRIFTED away.\n\n## Tail\n\nlater tail.\n";

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn said(out: &Output) -> String {
    format!(
        "status {:?}\nstdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
#[allow(clippy::too_many_lines)] // one sequential lifecycle script by design
fn repair_recovers_a_lost_cross_root_pin_from_the_target_roots_own_history() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let ws = tmp.path().join("ws");
    let other = tmp.path().join("other");
    for dir in [&home, &ws, &other] {
        std::fs::create_dir_all(dir).expect("mkdir");
    }

    // The ambient root: the pinning page's home. A git repo so the pin door
    // can mint (R4 demands a blob oid from the TARGET root, but the holder's
    // root needs no history here).
    std::fs::write(ws.join("plan.md"), "# Plan\n\ndraws from the other root.\n").expect("pinner");
    git(&ws, &["init", "-q"]);
    git(&ws, &["config", "user.email", "dg@example.invalid"]);
    git(&ws, &["config", "user.name", "dg"]);

    // The mounted target root, with its own declaration (INV-5) and history.
    std::fs::write(
        other.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: other\n---\n\n# Other root\n",
    )
    .expect("declaration");
    std::fs::write(other.join("doc.md"), DOC_V1).expect("target v1");
    git(&other, &["init", "-q"]);
    git(&other, &["config", "user.email", "dg@example.invalid"]);
    git(&other, &["config", "user.name", "dg"]);
    git(&other, &["add", "MERIDIAN.md"]);
    git(&other, &["commit", "-q", "-m", "seed"]);

    let config = home.join("MERIDIAN.md");
    std::fs::write(
        &config,
        format!(
            "---\ntype: meridian-config\nversion: 1\n---\n\n# Roots\n\n\
             ```meridian-mount\nname: other\npath: {}\n```\n",
            other.display()
        ),
    )
    .expect("mount table");
    // One env-dependent test binary by design (module docs). SAFETY: set
    // before any subprocess below reads it.
    unsafe { std::env::set_var("MERIDIAN_CONFIG", &config) };

    // An isolated cache home beside the isolated HOME: the registry drawer
    // and daemon socket derive from XDG_CACHE_HOME when it is set, and a
    // fleet shell exports it — inherited, the auto-spawned daemon refuses
    // ("another meridian registry daemon is already running for
    // ~/.cache/meridian/registry") and the pin never lands (measured on the
    // mac dev host, card mac-devhost-snapshot-canonicalization).
    let cache_home = tmp.path().join("xdg-cache");
    let run = |cwd: &Path, args: &[&str]| -> Output {
        Command::new(mrd_bin())
            .current_dir(cwd)
            .env("HOME", &home)
            .env("XDG_CACHE_HOME", &cache_home)
            .env("MERIDIAN_CONFIG", &config)
            .env_remove("MERIDIAN_WORKSPACE")
            .args(args)
            .output()
            .expect("spawn mrd")
    };

    // ① Mint the cross-root pin through the SHIPPED CLI door, --vibe so the
    //    whole-file blob (v1, uncommitted) lands loose in the TARGET store.
    let pin = run(
        &ws,
        &["pin", "plan.md", "other:doc.md#Doc/Design", "--vibe"],
    );
    assert_eq!(pin.status.code(), Some(0), "mrd pin: {}", said(&pin));
    let holder = std::fs::read_to_string(ws.join("plan.md")).expect("holder");
    assert!(
        holder.contains("object: \"[[other:doc]]\""),
        "the lock row carries the rooted spelling: {holder}"
    );
    let hash_before = holder
        .lines()
        .find_map(|l| l.trim().strip_prefix("hash: \""))
        .map(|l| l.trim_end_matches('"').to_owned())
        .expect("the minted row has a hash");

    // ② A later commit whose SECTION bytes are intact — the version repair
    //    must find — then drift the worktree and prune the loose blob.
    std::fs::write(other.join("doc.md"), DOC_V2).expect("target v2");
    git(&other, &["add", "doc.md"]);
    git(&other, &["commit", "-q", "-m", "v2 — section intact"]);
    std::fs::write(other.join("doc.md"), DOC_DRIFTED).expect("drift");
    git(&other, &["gc", "--prune=now", "-q"]);
    let holds = Command::new("git")
        .arg("-C")
        .arg(&other)
        .args(["cat-file", "-e", &hash_before])
        .status()
        .expect("git");
    assert!(
        !holds.success(),
        "the mint-time blob {hash_before} must be PRUNED for the pin to be lost"
    );

    // ③ `repair --dry` computes the recovery and writes nothing.
    let dry = run(&ws, &["repair", "--dry"]);
    assert_eq!(dry.status.code(), Some(0), "repair --dry: {}", said(&dry));
    assert_eq!(
        std::fs::read_to_string(ws.join("plan.md")).expect("holder"),
        holder,
        "--dry writes nothing"
    );

    // ④ The real repair rewrites `hash` alone — recovered from the TARGET
    //    root's own history (v2's blob), exit 0.
    let repair = run(&ws, &["repair"]);
    assert_eq!(repair.status.code(), Some(0), "repair: {}", said(&repair));
    let repaired = std::fs::read_to_string(ws.join("plan.md")).expect("holder");
    let hash_after = repaired
        .lines()
        .find_map(|l| l.trim().strip_prefix("hash: \""))
        .map(|l| l.trim_end_matches('"').to_owned())
        .expect("the repaired row has a hash");
    assert_ne!(hash_after, hash_before, "the retrieval plane moved");
    let anchored = Command::new("git")
        .arg("-C")
        .arg(&other)
        .args(["cat-file", "-e", &hash_after])
        .status()
        .expect("git");
    assert!(
        anchored.success(),
        "the recovered blob {hash_after} is held by the TARGET root's store"
    );
    // The forgery invariant across the crossing: everything but `hash` is
    // byte-identical.
    assert_eq!(
        holder.replace(&hash_before, &hash_after),
        repaired,
        "repair rewrote the hash and NOTHING else"
    );

    // ⑤ An UNMOUNTED root's pin stays skip-and-state — never walked, never
    //    guessed. `ghost` is bound nowhere; the run must still exit 0 (a
    //    stated skip is not a loss) and name the population.
    let ghost_pin = repaired.replace(
        "pins:\n",
        "pins:\n  - object: \"[[ghost:elsewhere]]\"\n    hash: \
         \"1111111111111111111111111111111111111111\"\n    path: [\"S\"]\n    fingerprint: \
         \"fp1.span2.b3.00000000\"\n",
    );
    std::fs::write(ws.join("plan.md"), &ghost_pin).expect("add ghost row");
    let stated = run(&ws, &["repair", "--json"]);
    assert_eq!(stated.status.code(), Some(0), "repair: {}", said(&stated));
    let json: serde_json::Value =
        serde_json::from_slice(&stated.stdout).expect("repair --json is machine-clean");
    let outside = json["out_of_jurisdiction"]
        .as_array()
        .expect("the stated population rides the json face");
    assert_eq!(outside.len(), 1, "one unmounted row stated: {json}");
    assert!(
        outside[0].as_str().is_some_and(|s| s.contains("ghost")),
        "and it names the root: {outside:?}"
    );
}
