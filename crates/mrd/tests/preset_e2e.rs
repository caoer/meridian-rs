//! U5.3 CLI wiring gate — drive the REAL `mrd` binary (`CARGO_BIN_EXE_mrd`) over its process
//! boundary for `mrd unfold` and `mrd new`: a marked workspace, a preset def on disk, the
//! operator-facing exit codes and birth receipts.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SESSION_PRESET: &str = r#"---
type: def
defines: session
root: SESSION.md
inputs:
  - "conventions/reviewer-not-owner/CHECK.md@rev-a"
---

# Properties ^properties

- `type` required
- `preset` required

# Template ^template

```record
---
type: session
preset: presets/session.md
id: {{id}}
---

# {{id}}
```

# Unfold

- SESSION.md
- tasks/index.md
- results/notes.md
"#;

const BROKEN_PRESET: &str = r#"---
type: def
defines: session
inputs:
  - "conventions/reviewer-not-owner/CHECK.md@rev-a"
---

# Properties ^properties

- `type` required
- `owner` required

# Template ^template

```record
---
type: session
id: {{id}}
---

# {{id}}
```

# Unfold

- SESSION.md
"#;

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
        Command::new(env!("CARGO_BIN_EXE_mrd"))
            .args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("spawn mrd")
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A git-anchored workspace carrying both preset defs.
fn workspace(sb: &Sandbox) -> PathBuf {
    let ws = sb.tmp.path().join("ws");
    std::fs::create_dir_all(ws.join("presets")).unwrap();
    // Anchored by a `.git` entry — the ladder's structural rung.
    std::fs::create_dir_all(ws.join(".git")).unwrap();
    std::fs::write(ws.join("presets/session.md"), SESSION_PRESET).unwrap();
    std::fs::write(ws.join("presets/broken.md"), BROKEN_PRESET).unwrap();
    ws
}

#[test]
fn cli_unfold_births_the_scaffold_and_new_refuses_an_invalid_def() {
    let sb = sandbox();
    let ws = workspace(&sb);

    // `mrd unfold` births every scaffold file (exit 0) with birth receipts.
    let out = sb.run(&ws, &["unfold", "session", "--actor", "op", "--now", "t0"]);
    assert!(out.status.success(), "unfold failed: {}", stderr(&out));
    for path in ["SESSION.md", "tasks/index.md", "results/notes.md"] {
        assert!(ws.join(path).exists(), "{path} not materialized");
    }
    // **The birth-receipt count is GONE, and no count replaces it.** This asserted three
    // `op=create` rows in `meridian/journal.md` — one per scaffold file — as the proof that each
    // birth went through the governed door. There is no journal: the engine keeps no memory , so a
    // write leaves no in-engine trace to count.
    //
    //
    //
    //
    //
    //
    //
    //
    assert!(
        !ws.join("meridian/journal.md").exists(),
        "and nothing mints a ledger any more — a page here would mean the journal \
         came back"
    );

    // A second unfold refuses every path via the if_absent CAS → exit 1.
    let again = sb.run(&ws, &["unfold", "session"]);
    assert_eq!(again.status.code(), Some(1), "re-unfold is a finding");

    // `mrd new` on an invalid def refuses def_invalid, naming the rule → exit 1.
    let bad = sb.run(&ws, &["new", "broken", "s1"]);
    assert_eq!(bad.status.code(), Some(1), "invalid def must refuse");
    let msg = stderr(&bad);
    assert!(
        msg.contains("def_invalid"),
        "names the taxonomy code: {msg}"
    );
    assert!(msg.contains("owner"), "names the def rule: {msg}");
    assert!(
        !ws.join("session/s1.md").exists(),
        "refused birth wrote a file"
    );

    // `mrd new` on the valid def births the first rev → exit 0.
    let ok = sb.run(
        &ws,
        &["new", "session", "s2", "--actor", "op", "--now", "t1"],
    );
    assert!(ok.status.success(), "valid new failed: {}", stderr(&ok));
    assert!(ws.join("session/s2.md").exists(), "record not born");
}
