//! TEMPORARY probe — dumps live v2 frames so the U27 key-set suite can be
//! written against reality. Deleted before the suite lands.

use serde_json::Value;
use std::io::Write as _;

fn workspace(files: &[(&str, &str)]) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, bytes) in files {
        let abs = dir.path().join(rel);
        std::fs::create_dir_all(abs.parent().expect("parent")).expect("mkdir");
        let mut f = std::fs::File::create(&abs).expect("create");
        f.write_all(bytes.as_bytes()).expect("write");
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn wsfix(rel: &str) -> String {
    std::fs::read_to_string(testsuite::wsfix_dir().join(rel))
        .unwrap_or_else(|e| panic!("wsfix fixture {rel}: {e}"))
}

fn s0() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let plan = wsfix("s0/notes/plan.md");
    let receipts = wsfix("s0/receipts/2026-07-18.md");
    workspace(&[
        ("notes/plan.md", &plan),
        ("receipts/2026-07-18.md", &receipts),
    ])
}

fn serve(root: &fs::WorkspaceRoot, input: &str) -> Vec<Value> {
    let mut out = Vec::new();
    sidecar::serve(root, input.as_bytes(), &mut out, &[]).expect("serve");
    String::from_utf8(out)
        .expect("utf8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("frame parses"))
        .collect()
}

const R0: &str = "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9";

#[test]
fn probe_dump_every_v2_frame() {
    let (_d, root) = s0();
    let input = format!(
        concat!(
            r#"{{"id":1,"op":"hello","proto":1,"client":"md-cli/0.3"}}"#,
            "\n",
            r#"{{"id":2,"op":"toc","path":"notes/plan.md"}}"#,
            "\n",
            r#"{{"id":3,"op":"cat","path":"notes/plan.md","sec":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}}}}"#,
            "\n",
            r#"{{"id":4,"op":"cat","path":"notes/plan.md"}}"#,
            "\n",
            r#"{{"id":5,"op":"extract","path":"notes/plan.md"}}"#,
            "\n",
            r#"{{"id":6,"op":"extract","path":"receipts/2026-07-18.md"}}"#,
            "\n",
            r#"{{"id":7,"op":"resolve","from":"notes/plan.md","ref":"plan#Goals#Q3"}}"#,
            "\n",
            r#"{{"id":8,"op":"resolve","from":"notes/plan.md","ref":"plan#Goals#Q3","content":true}}"#,
            "\n",
            r#"{{"id":9,"op":"links","path":"notes/plan.md"}}"#,
            "\n",
            r#"{{"id":10,"op":"links"}}"#,
            "\n",
            r#"{{"id":11,"op":"root"}}"#,
            "\n",
            r#"{{"id":12,"op":"sub","from_seq":0}}"#,
            "\n",
            r#"{{"id":13,"op":"splice","path":"notes/plan.md","dry":true,"edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"ship by August","new":"ship by September"}}}}}}]}}"#,
            "\n",
            r#"{{"id":14,"op":"splice","path":"notes/plan.md","actor":"agent:b0864fb2","now":"2026-07-18T20:31:04Z","receipt":{{"path":"receipts/2026-07-18.md","anchor":"r-000042"}},"if_root":"{r0}","edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"ship by August","new":"ship by September"}}}},"if_node_rev":"33d5b0e1b27cb48b"}}]}}"#,
            "\n",
            r#"{{"id":15,"op":"root"}}"#,
            "\n",
            r#"{{"id":16,"op":"splice","path":"notes/plan.md","edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"ship by August","new":"x"}}}},"if_node_rev":"33d5b0e1b27cb48b"}}]}}"#,
            "\n",
            r#"{{"id":17,"op":"splice","path":"notes/plan.md","edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q4"}}]}},"edit":{{"match":{{"old":"item","new":"entry"}}}}}}]}}"#,
            "\n",
            r#"{{"id":18,"op":"splice","path":"notes/plan.md","if_root":"{r0}","edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q4"}}]}},"edit":{{"match":{{"old":"item one","new":"entry"}}}}}}]}}"#,
            "\n",
            r#"{{"id":19,"op":"resolve","from":"notes/plan.md","ref":"plan#Goals#Q9"}}"#,
            "\n",
            r#"{{"id":20,"op":"resolve","from":"notes/plan.md","ref":"roadmap"}}"#,
            "\n",
            r#"{{"id":21,"op":"extract","path":"notes/plan.md","kinds":["bogus"]}}"#,
            "\n",
            r#"{{"id":22,"op":"nope"}}"#,
            "\n",
            r#"{{"id":23,"op":"toc","path":"missing.md"}}"#,
            "\n",
            r#"{{"id":24,"op":"links","path":"notes/plan.md","require_root":"{r0}"}}"#,
            "\n",
        ),
        r0 = R0
    );
    for f in serve(&root, &input) {
        println!("FRAME {}", serde_json::to_string(&f).expect("ser"));
    }
}

#[test]
fn probe_diff_and_delta() {
    let (_d, root) = s0();
    let input = format!(
        concat!(
            r#"{{"id":1,"op":"sub","from_seq":0}}"#,
            "\n",
            r#"{{"id":2,"op":"splice","path":"notes/plan.md","actor":"agent:b0864fb2","now":"2026-07-18T20:31:04Z","receipt":{{"path":"receipts/2026-07-18.md","anchor":"r-000042"}},"edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"ship by August","new":"ship by September"}}}}}}]}}"#,
            "\n",
            r#"{{"id":3,"op":"root"}}"#,
            "\n",
            r#"{{"id":4,"op":"diff","from_root":"{r0}","to_root":"{r0}"}}"#,
            "\n",
        ),
        r0 = R0
    );
    for f in serve(&root, &input) {
        println!("FRAME {}", serde_json::to_string(&f).expect("ser"));
    }
}
