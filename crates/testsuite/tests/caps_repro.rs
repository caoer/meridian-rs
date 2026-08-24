//! TEMPORARY reproduction probe — measures what each plane does with each
//! `caps:` spelling on pristine main. Deleted once the ONE-PARSER fix lands and
//! `caps_one_grammar.rs` replaces it. Uses only the public API that exists
//! BEFORE the fix, so it compiles against the faulting tree.

use policy::{CheckLimits, PageRef, ScopeLayer, register_page};

fn hook_page(caps_block: &str) -> String {
    format!(
        "---\n\
         tags: [type/rule, rules/hook]\n\
         id: caps.spelling-probe\n\
         severity: info\n\
         paths: [\"tasks/*.md\"]\n\
         {caps_block}\
         budget: {{ steps: 10000, mem: 4194304 }}\n\
         how:\n  \
         route:    {{ info: channel-review }}\n  \
         batching: 30s\n\
         ---\n\
         \n\
         # caps spelling probe\n\
         \n\
         ```starlark\n\
         def on_change(event):\n    \
         send(to = [\"reviewer\"], message = \"probe\")\n\
         ```\n"
    )
}

fn policy_caps(caps_block: &str) -> Result<Vec<String>, String> {
    let md = hook_page(caps_block);
    let registration = register_page(PageRef {
        layer: ScopeLayer::Workspace,
        page: "rules/caps-spelling-probe.md",
        bytes: &md,
    })
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "did not register".to_string())?;
    let rule =
        policy::load_rule(&registration, &md, CheckLimits::default()).map_err(|e| e.to_string())?;
    let hook = rule.hook().ok_or_else(|| "no hook leg".to_string())?;
    Ok(hook.caps().iter().map(|k| k.as_str().to_string()).collect())
}

fn run_caps(caps_block: &str) -> Result<Option<Vec<String>>, String> {
    let raw = format!("---\ntype: hooks\n{caps_block}---\n\n# probe\n");
    let document = model::build(raw.clone(), syntax::parse(&raw));
    match run::caps::page_caps(&document) {
        Ok(v) => Ok(v.map(|set| set.0.iter().map(run::caps::Cap::as_string).collect())),
        Err(e) => Err(e.to_string()),
    }
}

#[test]
fn measure_every_spelling_on_both_planes() {
    let cases: [(&str, String, String); 5] = [
        (
            "plain scalar",
            "caps: proto.send\n".into(),
            "caps: md.create, md.edit\n".into(),
        ),
        (
            "flow sequence",
            "caps: [proto.send]\n".into(),
            "caps: [md.create, md.edit]\n".into(),
        ),
        (
            "flow sequence, quoted items",
            "caps: [\"proto.send\"]\n".into(),
            "caps: [\"md.create\", \"md.edit\"]\n".into(),
        ),
        (
            "block sequence",
            "caps:\n  - proto.send\n".into(),
            "caps:\n  - md.create\n  - md.edit\n".into(),
        ),
        (
            "declared empty",
            "caps: []\n".into(),
            "caps: []\n".into(),
        ),
    ];

    let mut report = String::from("\n=== caps spelling matrix (pristine main) ===\n");
    for (name, policy_block, run_block) in &cases {
        report.push_str(&format!("\n--- {name}\n"));
        report.push_str(&format!("  policy: {:?}\n", policy_caps(policy_block)));
        report.push_str(&format!("  run   : {:?}\n", run_caps(run_block)));
    }
    report.push_str("\n--- bare `caps:` (key present, nothing after it)\n");
    report.push_str(&format!("  policy: {:?}\n", policy_caps("caps:\n")));
    report.push_str(&format!("  run   : {:?}\n", run_caps("caps:\n")));
    report.push_str("\n--- absent `caps:`\n");
    report.push_str(&format!("  policy: {:?}\n", policy_caps("")));
    report.push_str(&format!("  run   : {:?}\n", run_caps("")));

    println!("{report}");
    panic!("repro probe — matrix printed above");
}
