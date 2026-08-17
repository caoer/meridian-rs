//! The rules walk — the disk edge that feeds tag-indexed registration.
//!
//! `policy` is the WHEN plane and is I/O-free by charter (docs/laws.md), so [`policy::RuleIndex`]
//! reads a caller-supplied page feed. This module is that caller: it enumerates the ladder's roots
//! on disk, reads each page, and offers it to the index one page at a time, so registering a vault
//! costs one page of memory rather than the whole corpus. The door never walks — it reads the one
//! attested armed-set artifact. Walking is the sweep/inspect side, so it sits beside the CLI verbs.

use std::io;
use std::path::{Path, PathBuf};

use fs::WorkspaceRoot;
use fs::domain::Domain;
use policy::{PageRef, RuleIndex, ScopeLayer};

/// The `MERIDIAN.md` the user rung is anchored on, resolved through the bootstrap chain
/// (`MERIDIAN_CONFIG`, else `$HOME/MERIDIAN.md`). This resolves the ANCHOR only. Whether it
/// exists, what lives beside it, and what counts as a user rule page are all
/// [`fs::user_rule_pages`]'s law — one function for the whole rung, called by this walk and by
/// `mrd rules`, never forked into two enumerations that could disagree.
#[must_use]
pub fn user_anchor(env: &config::Env) -> Option<PathBuf> {
    config::resolve_path(env).ok()
}

/// One page the walk could not offer. Reading a vault touches files that are unreadable or not
/// UTF-8; neither is a registration verdict, and neither may un-register the rest of the
/// workspace, so they are collected rather than raised.
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
    /// The discovery index — every page that registered, plus every page that offered itself and
    /// was refused. Call [`policy::RuleIndex::resolve`] on it (after narrowing, if the consumer
    /// narrows).
    #[must_use]
    pub fn index(&self) -> &RuleIndex {
        &self.index
    }

    /// The index, consumed.
    #[must_use]
    pub fn into_index(self) -> RuleIndex {
        self.index
    }

    /// The roots this walk enumerated, outermost rung first. A layer whose root did not exist is
    /// absent here — what was walked is a fact the caller can print.
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
/// `user_anchor` is the user rung's `MERIDIAN.md` ([`user_anchor`]); pass `None` to
/// walk the workspace alone. The rung's whole law — an absent anchor is an empty
/// layer, `<user-scope>/rules/**.md` only, never a `$HOME` walk — belongs to
/// [`fs::user_rule_pages`], which this walk calls rather than reimplements.
///
/// A user rung nested inside the workspace root is skipped rather than walked
/// twice: those pages are already workspace pages, and offering them under both
/// layers would invent a same-id collision out of one page.
///
/// # Errors
/// I/O failure enumerating a root that exists, or a workspace carrying two domain
/// configs ([`fs::domain::Domain::load`] refuses to guess which is in force).
pub fn walk_rules(workspace_root: &Path, user_anchor: Option<&Path>) -> io::Result<RulesWalk> {
    let mut walk = RulesWalk {
        index: RuleIndex::default(),
        roots: Vec::new(),
        unreadable: Vec::new(),
    };

    if let Some(anchor) = user_anchor
        && let Some(user_scope) = anchor.parent()
        && !user_scope
            .join(fs::USER_RULES_DIR)
            .starts_with(workspace_root)
    {
        let pages = fs::user_rule_pages(anchor)?;
        if !pages.is_empty() {
            walk.roots
                .push((ScopeLayer::User, user_scope.to_path_buf()));
        }
        for (page, bytes) in pages {
            match String::from_utf8(bytes) {
                Ok(bytes) => walk.index.offer(PageRef {
                    layer: ScopeLayer::User,
                    page: &page,
                    bytes: &bytes,
                }),
                Err(e) => walk.unreadable.push(UnreadablePage {
                    page,
                    layer: ScopeLayer::User,
                    error: io::Error::new(io::ErrorKind::InvalidData, e),
                }),
            }
        }
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
        // Page paths are the addressing the arm artifact and the print verb render, so they are
        // `/`-separated regardless of the platform's separator.
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

/// Split excluded pages into REGISTRATION CANDIDATES and pages whose rule-ness
/// CANNOT BE ANSWERED, by asking the registrar — never a path predicate here,
/// which would be a fork of `policy`'s law that could disagree with it.
///
/// ⛔ THERE ARE THREE STATES, NOT TWO, and collapsing them shipped wrong twice
/// (`rules_cmd`'s history): `RegisterFault::FrontmatterUnparsed` says in its own
/// words that whether the page carries a registration tag *cannot be
/// answered*; EVERY OTHER fault variant presupposes the tag was there. So a
/// page with a broken frontmatter block is neither a dropped rule page nor
/// established not to be one — it gets its own verdict list.
///
/// A candidate is `(page, Some(id))` when the page would have REGISTERED —
/// the id an ARM refusal can be matched against — and `(page, None)` when it
/// offered itself and was refused for anything but an unparseable frontmatter
/// block. One narrowing law for every excluded feed: the custom-ignore class,
/// the workspace dot class, and the user rung's dot decline
/// (card rules-silent-nonregistration).
#[must_use]
pub fn rule_candidates_among(
    candidates: &[(String, String)],
) -> (Vec<(String, Option<policy::RuleId>)>, Vec<String>) {
    let index = RuleIndex::discover(candidates.iter().map(|(page, bytes)| PageRef {
        layer: ScopeLayer::Workspace,
        page,
        bytes,
    }));
    let mut offered: Vec<(String, Option<policy::RuleId>)> = index
        .registered()
        .iter()
        .map(|r| (r.page().to_owned(), Some(r.id().clone())))
        .collect();
    let mut undecidable = Vec::new();
    for refusal in index.refused() {
        if matches!(
            refusal.fault(),
            policy::RegisterFault::FrontmatterUnparsed { .. }
        ) {
            undecidable.push(refusal.page().to_owned());
        } else {
            offered.push((refusal.page().to_owned(), None));
        }
    }
    offered.sort_by(|a, b| a.0.cmp(&b.0));
    offered.dedup_by(|a, b| a.0 == b.0);
    undecidable.sort();
    undecidable.dedup();
    (offered, undecidable)
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

    /// A user scope: the `MERIDIAN.md` anchor plus its `rules/` folder.
    fn user_scope(at: &Path, pages: &[(&str, &str)]) -> PathBuf {
        put(at, "MERIDIAN.md", "---\ntype: meridian-root\n---\n");
        for (rel, id) in pages {
            put(at, &format!("rules/{rel}"), &page(id));
        }
        at.join("MERIDIAN.md")
    }

    #[test]
    fn all_three_ladder_layers_are_offered_unnarrowed() {
        let tmp = tempfile::tempdir().expect("tmp");
        let anchor = user_scope(&tmp.path().join("user-scope"), &[("notify.md", "shared")]);
        let ws = tmp.path().join("workspace");
        put(&ws, "rules.md", &page("shared"));
        put(&ws, "sessions/s1/rules.md", &page("shared"));

        let walk = walk_rules(&ws, Some(&anchor)).expect("walks");
        assert_eq!(
            walk.index().registered().len(),
            3,
            "every candidate is offered, not just the winner"
        );

        // The chain: winner first, then outward to user space.
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
                // Spelled relative to the user scope — what `fs::user_rule_pages` returns.
                ("rules/notify.md", ScopeLayer::User),
            ]
        );
    }

    #[test]
    fn a_user_rung_inside_the_workspace_is_not_walked_twice() {
        let tmp = tempfile::tempdir().expect("tmp");
        let ws = tmp.path();
        let anchor = user_scope(ws, &[("notify.md", "shared")]);

        let walk = walk_rules(ws, Some(&anchor)).expect("walks");
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

    /// `MERIDIAN.md` declares the user scope. Without it the walk invents no layer: no user root
    /// is reported, and a `rules/` folder beside the absent anchor registers nothing.
    #[test]
    fn an_absent_meridian_md_anchor_is_an_empty_user_layer() {
        let tmp = tempfile::tempdir().expect("tmp");
        let home = tmp.path().join("home");
        let ws = tmp.path().join("workspace");
        put(&home, "rules/notify.md", &page("user.rule"));
        put(&ws, "rules.md", &page("workspace.rule"));

        let walk = walk_rules(&ws, Some(&home.join("MERIDIAN.md"))).expect("walks");
        assert_eq!(
            walk.roots().len(),
            1,
            "no anchor ⇒ no user layer, even with rules sitting right there"
        );
        assert_eq!(walk.roots()[0].0, ScopeLayer::Workspace);
        assert!(
            walk.index().resolve().get("user.rule").is_none(),
            "an unanchored rules/ folder is not a user scope"
        );

        // …and the same directory WITH the anchor present is one.
        write(home.join("MERIDIAN.md"), "---\ntype: meridian-root\n---\n").expect("write");
        let walk = walk_rules(&ws, Some(&home.join("MERIDIAN.md"))).expect("walks");
        assert_eq!(walk.roots().len(), 2);
        assert_eq!(
            walk.index()
                .resolve()
                .get("user.rule")
                .expect("the anchor declares the scope its rules sit beside")
                .winner()
                .scope()
                .layer(),
            ScopeLayer::User
        );
    }

    #[test]
    fn an_absent_user_rung_is_an_empty_layer_not_a_failure() {
        let tmp = tempfile::tempdir().expect("tmp");
        let ws = tmp.path();
        put(ws, "rules/notify.md", &page("x"));

        let walk = walk_rules(ws, Some(&tmp.path().join("nowhere/MERIDIAN.md"))).expect("walks");
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

    /// The three-state split: registered candidates carry their id, refused
    /// ones offer themselves without one, and an unparseable frontmatter block
    /// is undecidable — never claimed as either.
    #[test]
    fn rule_candidates_among_splits_the_three_states() {
        let candidates = vec![
            (".hidden/rules/x.md".to_owned(), page("mw.hidden-law")),
            (
                ".hidden/anonymous.md".to_owned(),
                "---\ntags: [rules/hook]\n---\n\n# no id\n".to_owned(),
            ),
            (
                ".hidden/broken.md".to_owned(),
                "---\ntags: [rules/hook\n---\n\n# unclosed flow sequence\n".to_owned(),
            ),
            (
                ".hidden/plain.md".to_owned(),
                "---\ntype: note\n---\n\n# not a rule\n".to_owned(),
            ),
        ];
        let (offered, undecidable) = rule_candidates_among(&candidates);
        assert_eq!(
            offered
                .iter()
                .map(|(page, id)| (page.as_str(), id.as_ref().map(policy::RuleId::as_str)))
                .collect::<Vec<_>>(),
            vec![
                (".hidden/anonymous.md", None),
                (".hidden/rules/x.md", Some("mw.hidden-law")),
            ],
            "candidates offer themselves; a registered one carries its id"
        );
        assert_eq!(
            undecidable,
            vec![".hidden/broken.md"],
            "unparseable frontmatter is its own verdict"
        );
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
