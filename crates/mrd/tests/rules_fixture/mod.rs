//! Disk fixtures shared by rules correctness and isolated CPU tests.
//! Each test binary uses a different subset of the helpers.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Output;

use crate::common;

/// A rule page: the registration tag, the id, a body that makes the bytes (and
/// so the rev) unique per page, and enough DECLARATION for the page to load.
///
/// The declaration half is not decoration. The ARM act loads the winner it
/// attests (card `mrd-arm-loads-page`), so a fixture the act can pin has to be
/// a page the loader accepts — a tag and an `id:` alone register and then
/// refuse, which is the very defect that gate exists to catch. Each kind gets
/// the keys and the entry point its own leg is evaluated through.
pub(crate) fn rule_page(kind: &str, id: &str, body: &str) -> String {
    let (keys, entry) = match kind {
        "hook" => (
            "severity: info\npaths: [\"**\"]\ncaps: []\n",
            "def on_change(event):\n    pass\n",
        ),
        "middleware" => ("paths: [\"**\"]\n", "def middleware(ctx):\n    pass\n"),
        _ => ("paths: [\"**\"]\n", "def check_change(change):\n    pass\n"),
    };
    format!(
        "---\ntags: [type/rule, rules/{kind}]\nid: {id}\n{keys}---\n\n# {id}\n\n{body}\n\n\
         ```starlark\n{entry}```\n"
    )
}

pub(crate) struct Sandbox {
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    pub(crate) home: PathBuf,
    pub(crate) cache_home: PathBuf,
    pub(crate) ws: PathBuf,
}

impl Sandbox {
    pub(crate) fn write(&self, rel: &str, bytes: &str) {
        let path = self.ws.join(rel);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }

    pub(crate) fn write_home(&self, rel: &str, bytes: &str) {
        let path = self.home.join(rel);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }

    pub(crate) fn run(&self, args: &[&str]) -> Output {
        common::mrd_command(&self.home, &self.cache_home)
            .args(args)
            .current_dir(&self.ws)
            .env_remove("MERIDIAN_CONFIG")
            .env_remove("MERIDIAN_WORKSPACE")
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .output()
            .expect("spawn mrd")
    }

    /// stdout, with a non-zero exit's stderr attached to the panic message.
    pub(crate) fn stdout(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "mrd {args:?} exited {:?}\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("utf-8 stdout")
    }

    /// Every file under the workspace as `(relative path, bytes)` — the read-only witness. Wider
    /// than the hash domain on purpose: a verb that wrote a marker, a lock, or a dotfile would
    /// escape a domain-only compare.
    pub(crate) fn tree(&self) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        collect(&self.ws, &self.ws, &mut out);
        out
    }

    /// The production hash-domain fold over the workspace — the same root the
    /// engine's own guards compare.
    pub(crate) fn merkle_root(&self) -> String {
        let root = fs::WorkspaceRoot(self.ws.clone());
        let (_files, folded) = fs::domain_snapshot(&root).expect("snapshot");
        format!("{folded:?}")
    }
}

fn collect(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let entry = entry.expect("entry");
        let path = entry.path();
        if entry.file_type().expect("file_type").is_dir() {
            collect(base, &path, out);
        } else {
            let rel = path
                .strip_prefix(base)
                .expect("under the base")
                .to_string_lossy()
                .to_string();
            out.insert(rel, std::fs::read(&path).expect("read"));
        }
    }
}

/// A sandbox whose workspace is a declared meridian root and whose HOME carries
/// a `MERIDIAN.md` anchor (so the user rung is declared) but no `rules/` yet.
pub(crate) fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let cache_home = tmp.path().join("xdg-cache");
    let ws = tmp.path().join("ws");
    for dir in [&home, &cache_home, &ws] {
        std::fs::create_dir_all(dir).expect("mkdir");
    }
    let sandbox = Sandbox {
        tmp,
        home,
        cache_home,
        ws,
    };
    sandbox.write(
        "MERIDIAN.md",
        "---\ntype: meridian-root\nversion: 1\nname: ws\n---\n\n# The workspace\n",
    );
    sandbox.write_home(
        "MERIDIAN.md",
        "---\ntype: meridian-config\nversion: 1\n---\n\n# This machine\n",
    );
    sandbox
}

/// Mint a real ARM artifact over the fixtures own bytes and write it into the workspace.
/// `requests` is `(arm root, id, mode)`; every winners rev is pinned by `policy::armed::arm`
/// through the landed resolver.
pub(crate) fn arm(s: &Sandbox, requests: &[(&str, &str, &str)]) {
    let root = fs::WorkspaceRoot(s.ws.clone());
    let (files, _) = fs::domain_snapshot(&root).expect("snapshot");
    let text: Vec<(String, String)> = files
        .into_iter()
        .map(|(page, bytes)| (page, String::from_utf8(bytes).expect("utf-8")))
        .collect();
    let index = policy::RuleIndex::discover(text.iter().map(|(page, bytes)| policy::PageRef {
        layer: policy::ScopeLayer::Workspace,
        page,
        bytes,
    }));

    // The act loads every firing winner — hand it the snapshot's own bytes, so
    // the loader reads exactly what the resolver resolved.
    let source: BTreeMap<String, String> = text.iter().cloned().collect();

    let mut artifact: Option<policy::armed::ArmedArtifact> = None;
    for (arm_root, id, mode) in requests {
        let root = policy::armed::ArmRoot::parse(arm_root).expect("a legal root");
        // The rev the "reviewer" attests is the winner's own, read back through
        // the same resolver the arm act will use — never a literal.
        let resolved = index.narrowed_to(root.as_str()).resolve();
        let attested_rev = resolved
            .get(id)
            .expect("the id resolves at this root")
            .winner()
            .rev()
            .to_owned();
        let act = policy::armed::arm(
            &index,
            &root,
            vec![policy::armed::ArmRequest {
                id: policy::RuleId::parse(id).expect("a legal id"),
                mode: policy::armed::Mode::parse(mode).expect("a legal mode"),
                attested_rev,
            }],
            &source,
            policy::CheckLimits::default(),
        )
        .expect("the arm act");
        match artifact.as_mut() {
            None => artifact = Some(act),
            Some(held) => held.merge(act).expect("merge"),
        }
    }
    let page = artifact.expect("at least one arm").render();
    s.write(policy::armed::ARMED_RULES_PATH, &page);
}
