//! Print the relative path of every hash-domain member, one per line — the
//! member list `tools/bench-root-2x.sh` constructs and stamps bench roots
//! from. The list folds through the same [`fs::hash_domain`] the daemon uses,
//! so the rig's notion of the domain is the engine's own and never a `find`
//! approximation — the drift that made nested-copy bench roots misreport in
//! the first place.
//!
//! ```text
//! cargo run --release -p fs --example domain_members -- <root>
//! ```

fn main() {
    let root_arg = std::env::args()
        .nth(1)
        .expect("usage: domain_members <root>");
    let root = fs::WorkspaceRoot(std::path::PathBuf::from(&root_arg));
    let domain = fs::domain::Domain::load(&root).expect("domain");
    let rels = fs::hash_domain(&root, &domain).expect("walk");
    let mut out = String::new();
    for rel in &rels {
        out.push_str(&rel.display().to_string());
        out.push('\n');
    }
    print!("{out}");
}
