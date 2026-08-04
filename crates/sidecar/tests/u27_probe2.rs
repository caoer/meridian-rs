//! TEMPORARY probe 2 — error frames + anchor toc row. Deleted before landing.

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

#[test]
fn probe_errors() {
    let (_d, root) = s0();
    let q3 = "33d5b0e1b27cb48b";
    let q4 = "4b8bc385a58da0e0";
    let input = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        format_args!(
            r#"{{"id":1,"op":"splice","path":"notes/plan.md","edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"nope nope","new":"x"}}}},"if_node_rev":"{q3}"}}]}}"#
        ),
        format_args!(
            r#"{{"id":2,"op":"splice","path":"notes/plan.md","edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q4"}}]}},"edit":{{"match":{{"old":"item","new":"entry"}}}},"if_node_rev":"{q4}"}}]}}"#
        ),
        format_args!(
            r#"{{"id":3,"op":"splice","path":"notes/plan.md","edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"ship","new":"a"}}}},"if_node_rev":"{q3}"}},{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"August","new":"b"}}}},"if_node_rev":"{q3}"}}]}}"#
        ),
        r#"{"id":"7","op":"root"}"#,
        r#"{"id":5,"op":"hello","proto":99}"#,
        r#"{"id":6,"op":"toc","path":"../escape.md"}"#,
        r#"{"id":7,"op":"resolve","from":"notes/plan.md","ref":"plan#Goals"}"#,
        format_args!(
            r#"{{"id":8,"op":"splice","path":"notes/plan.md","actor":"a","now":"2026-07-18T20:31:04Z","receipt":{{"path":"receipts/2026-07-18.md","anchor":"r-000042"}},"edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"ship by August","new":"ship by September"}}}},"if_node_rev":"{q3}"}}]}}"#
        ),
        r#"{"id":9,"op":"toc","path":"receipts/2026-07-18.md"}"#,
    );
    for f in serve(&root, &input) {
        println!("FRAME {}", serde_json::to_string(&f).expect("ser"));
    }
}
