//! The rules walk — the disk edge that feeds tag-indexed registration.
//!
//! # Why this lives here and not in `policy`
//! `policy` is the WHEN plane and is I/O-free by charter (docs/laws.md), so
//! [`policy::RuleIndex`] reads a caller-supplied page feed. This module is that
//! caller: it enumerates the ladder's roots on disk, reads each page, and offers it
//! to the index one page at a time ([`policy::RuleIndex::offer`]) so registering a
//! vault costs one page of memory rather than the whole corpus.
//!
//! # Why the CLI is the caller, and the door is not
//! The door never walks. It reads the ONE attested armed-set artifact and takes its
//! rows (`policy::index`'s O(armed) law) — re-walking to learn the armed set is
//! exactly what attestation replaces. Walking is the SWEEP/inspect side: what is
//! registered here, what shadows what. Those are CLI verbs, so the walk sits beside
//! them.
//!
//! # The ladder (ruling § 3)
//! Outermost to innermost: **user space** → **workspace root** → **folder/session
//! tree**. The last two are ONE layer separated by depth — a session-tree page is
//! simply a deeper workspace page — so the walk enumerates two roots, and
//! [`policy::Scope`] separates the inner two by mount depth.
//!
//! # The walk offers EVERY candidate, and narrows nothing
//! Resolution is narrowed to the pages mounted at-or-above the evaluated path
//! (ruling § 3, amended 2026-08-01), and narrowing is a CONSUMER step through
//! [`policy::Registration::mount_dir`] before `resolve()`. A walk that narrowed
//! would make the override chain unprintable (§ 7) — the shadowed candidates it
//! dropped are exactly what `mrd rules` must show. So this walk gathers all three
//! layers whole, and every consumer narrows for itself.
//!
//! # A rule page lives in the hash domain
//! Pages are enumerated through [`fs::hash_domain`], not [`fs::walk`]: dot-directory
//! files are addressable but sit OUTSIDE the hash domain, and law that cannot be
//! hashed cannot be attested (the same rule the convention loader spells as "never a
//! dot-dir"). A page the domain excludes is not a rule page.

use std::io;
use std::path::{Path, PathBuf};

use fs::WorkspaceRoot;
use fs::domain::Domain;
use policy::{PageRef, RuleIndex, ScopeLayer};

/// The directory the user-space layer's rule pages live in, beside the resolved
/// `MERIDIAN.md` (`MERIDIAN_CONFIG`, else `$HOME/MERIDIAN.md`).
///
/// The rung is bounded to `rules/` DELIBERATELY. "Rules under the user scope,
/// sibling of `~/MERIDIAN.md`" (ruling § 3) names a scope, not a walk instruction:
/// read as the config file's whole directory it would enumerate `$HOME`, which is
/// neither a corpus nor bounded. One conventional directory, named for the
/// registration namespace itself, keeps the rung real without walking a home
/// directory to find three pages.
pub const USER_RULES_DIR: &str = "rules";

/// Where the user-space layer is rooted, or `None` when the bootstrap chain names
/// no config (so there is no user scope to be a sibling of).
///
/// The directory need not exist — an absent `rules/` folder is an empty layer, and
/// [`walk_rules`] treats it as one.
#[must_use]
pub fn user_rules_root(env: &config::Env) -> Option<PathBuf> {
    let config_path = config::resolve_path(env).ok()?;
    Some(config_path.parent()?.join(USER_RULES_DIR))
}

/// One page the walk could not offer. Reading a vault touches files that are
/// unreadable or not UTF-8; neither is a registration verdict, and neither may
/// un-register the rest of the workspace, so they are collected rather than raised.
#[derive(Debug)]
pub struct UnreadablePage {
    /// The page's path, relative to its layer's root.
    pub page: String,
    /// Which rung it was found on.
    pub layer: ScopeLayer,
    /// Why it could not be read.
    pub error: io::Error,
}

/// What one walk found: the discovery index, the roots it was built from, and the
/// pages it could not read.
#[derive(Debug)]
pub struct RulesWalk {
    index: RuleIndex,
    roots: Vec<(ScopeLayer, PathBuf)>,
    unreadable: Vec<UnreadablePage>,
}

impl RulesWalk {
    /// The discovery index — every page that registered, plus every page that
    /// offered itself and was refused. Call [`policy::RuleIndex::resolve`] on it
    /// (after narrowing, if the consumer narrows).
    #[must_use]
    pub fn index(&self) -> &RuleIndex {
        &self.index
    }

    /// The index, consumed.
    #[must_use]
    pub fn into_index(self) -> RuleIndex {
        self.index
    }

    /// The roots this walk enumerated, outermost rung first. A layer whose root did
    /// not exist is absent here — what was walked is a fact the caller can print.
    #[must_use]
    pub fn roots(&self) -> &[(ScopeLayer, PathBuf)] {
        &self.roots
    }

    /// Pages that could not be read. Never a registration refusal — those live in
    /// [`policy::RuleIndex::refused`].
    #[must_use]
    pub fn unreadable(&self) -> &[UnreadablePage] {
        &self.unreadable
    }
}

/// Walk the ladder and offer every markdown page in the hash domain of each root to
/// tag registration.
///
/// `user_rules_root` is the user-space rung ([`user_rules_root`]); pass `None` to
/// walk the workspace alone. A root that does not exist is an empty layer, not a
/// failure — a machine with no user rules is the common case, not a broken one.
///
/// A user rung nested inside the workspace root is SKIPPED rather than walked
/// twice: those pages are already workspace pages, and offering them under both
/// layers would invent a same-id collision out of one page.
///
/// # Errors
/// I/O failure enumerating a root that exists, or a workspace carrying two domain
/// configs ([`fs::domain::Domain::load`] refuses to guess which is in force).
pub fn walk_rules(workspace_root: &Path, user_rules_root: Option<&Path>) -> io::Result<RulesWalk> {
    let mut walk = RulesWalk {
        index: RuleIndex::default(),
        roots: Vec::new(),
        unreadable: Vec::new(),
    };

    if let Some(user_root) = user_rules_root
        && user_root.exists()
        && !user_root.starts_with(workspace_root)
    {
        offer_root(&mut walk, ScopeLayer::User, user_root, &Domain::new())?;
    }

    if workspace_root.exists() {
        let root = WorkspaceRoot(workspace_root.to_path_buf());
        let domain = Domain::load(&root)?;
        offer_root(&mut walk, ScopeLayer::Workspace, workspace_root, &domain)?;
    }

    Ok(walk)
}

/// Enumerate one root's hash domain and offer each page to the index.
fn offer_root(
    walk: &mut RulesWalk,
    layer: ScopeLayer,
    root: &Path,
    domain: &Domain,
) -> io::Result<()> {
    let workspace_root = WorkspaceRoot(root.to_path_buf());
    let pages = fs::hash_domain(&workspace_root, domain)?;
    walk.roots.push((layer, root.to_path_buf()));

    for rel in pages {
        // Page paths are the addressing the ARM artifact and the print verb render,
        // so they are `/`-separated regardless of the platform's separator.
        let page = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        match std::fs::read_to_string(root.join(&rel)) {
            Ok(bytes) => walk.index.offer(PageRef {
                layer,
                page: &page,
                bytes: &bytes,
            }),
            Err(error) => walk.unreadable.push(UnreadablePage { page, layer, error }),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{create_dir_all, write};

    /// A minimal rule page: registration tag + id.
    fn page(id: &str) -> String {
        format!("---\ntags: [type/rule, rules/hook]\nid: {id}\n---\n\n# rule\n")
    }

    fn put(root: &Path, rel: &str, body: &str) {
        let path = root.join(rel);
        create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        write(path, body).expect("write");
    }

    #[test]
    fn the_walk_registers_tagged_pages_and_ignores_ordinary_ones() {
        let tmp = tempfile::tempdir().expect("tmp");
        let ws = tmp.path();
        put(ws, "rules/notify.md", &page("task.review-notify"));
        put(ws, "notes/ordinary.md", "---\ntype: note\n---\n\n# note\n");
        put(ws, "README.md", "# no frontmatter\n");

        let walk = walk_rules(ws, None).expect("walks");
        let pages: Vec<&str> = walk
            .index()
            .registered()
            .iter()
            .map(policy::Registration::page)
            .collect();
        assert_eq!(pages, vec!["rules/notify.md"]);
        assert!(walk.index().refused().is_empty());
        assert!(walk.unreadable().is_empty());
    }

    #[test]
    fn all_three_ladder_layers_are_offered_unnarrowed() {
        let tmp = tempfile::tempdir().expect("tmp");
        let user = tmp.path().join("user-scope/rules");
        let ws = tmp.path().join("workspace");
        put(&user, "notify.md", &page("shared"));
        put(&ws, "rules.md", &page("shared"));
        put(&ws, "sessions/s1/rules.md", &page("shared"));

        let walk = walk_rules(&ws, Some(&user)).expect("walks");
        assert_eq!(
            walk.index().registered().len(),
            3,
            "every candidate is offered, not just the winner"
        );

        // The chain the print verb needs is reconstructable: winner first, then
        // outward to user space.
        let set = walk.index().resolve();
        let chain: Vec<(&str, ScopeLayer)> = set
            .get("shared")
            .expect("resolves")
            .chain()
            .map(|r| (r.page(), r.scope().layer()))
            .collect();
        assert_eq!(
            chain,
            vec![
                ("sessions/s1/rules.md", ScopeLayer::Workspace),
                ("rules.md", ScopeLayer::Workspace),
                ("notify.md", ScopeLayer::User),
            ]
        );
    }

    #[test]
    fn a_user_rung_inside_the_workspace_is_not_walked_twice() {
        let tmp = tempfile::tempdir().expect("tmp");
        let ws = tmp.path();
        let user = ws.join("rules");
        put(ws, "rules/notify.md", &page("shared"));

        let walk = walk_rules(ws, Some(&user)).expect("walks");
        assert_eq!(
            walk.index().registered().len(),
            1,
            "one page on disk is one candidate, never a collision with itself"
        );
        assert!(
            walk.index().resolve().collisions().is_empty(),
            "and it still resolves"
        );
    }

    #[test]
    fn an_absent_user_rung_is_an_empty_layer_not_a_failure() {
        let tmp = tempfile::tempdir().expect("tmp");
        let ws = tmp.path();
        put(ws, "rules/notify.md", &page("x"));

        let walk = walk_rules(ws, Some(&tmp.path().join("nowhere/rules"))).expect("walks");
        assert_eq!(walk.roots().len(), 1);
        assert_eq!(walk.roots()[0].0, ScopeLayer::Workspace);
    }

    #[test]
    fn dot_directory_pages_are_outside_the_hash_domain_and_do_not_register() {
        let tmp = tempfile::tempdir().expect("tmp");
        let ws = tmp.path();
        put(ws, ".github/rules.md", &page("hidden"));
        put(ws, "rules.md", &page("visible"));

        let walk = walk_rules(ws, None).expect("walks");
        let pages: Vec<&str> = walk
            .index()
            .registered()
            .iter()
            .map(policy::Registration::page)
            .collect();
        assert_eq!(
            pages,
            vec!["rules.md"],
            "law that cannot be hashed cannot be attested"
        );
    }

    #[test]
    fn a_refused_page_does_not_un_register_the_rest() {
        let tmp = tempfile::tempdir().expect("tmp");
        let ws = tmp.path();
        put(ws, "good.md", &page("good"));
        put(ws, "anonymous.md", "---\ntags: [rules/hook]\n---\n");

        let walk = walk_rules(ws, None).expect("walks");
        assert_eq!(walk.index().registered().len(), 1);
        assert_eq!(walk.index().refused().len(), 1);
        assert_eq!(walk.index().refused()[0].page(), "anonymous.md");
    }

    #[test]
    fn a_page_that_is_not_utf8_is_recorded_not_raised() {
        let tmp = tempfile::tempdir().expect("tmp");
        let ws = tmp.path();
        put(ws, "good.md", &page("good"));
        write(ws.join("binary.md"), [0xff, 0xfe, 0x00]).expect("write");

        let walk = walk_rules(ws, None).expect("an unreadable page is not a walk failure");
        assert_eq!(walk.index().registered().len(), 1);
        assert_eq!(walk.unreadable().len(), 1);
        assert_eq!(walk.unreadable()[0].page, "binary.md");
    }
}
