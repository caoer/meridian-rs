//! `mrd move` — the move door (`docs/move.md`).
//!
//! ```text
//! mrd move <OLD> <NEW> [--dry] [--immutable PREFIX]... [--json]
//! ```
//!
//! A thin client of `wire_serve::relocate`: argument parsing, the rooted lane
//! on both operands (one root, or `cross_root` refuses), the §1 path law at
//! exit 2, workspace resolution, and the two output faces. Every decision —
//! what would break, what the new spelling is, the ambiguity refusal, the
//! immutable skips, the crash order — lives in the engine (`docs/laws.md`).
//!
//! Exit triad: 0 moved (or a dry plan that would move) / 1 refused — every
//! engine refusal, the ambiguity class, a read-back the plan did not predict /
//! 2 bad invocation.

use std::path::Path;

use serde_json::{Value, json};
use wire_serve::relocate::{RelocateArgs, RelocateOutcome, ambiguity_refusal, relocate};

use crate::{Fail, Format, current_dir, engine};

const CONSEQUENCE: &str = "Nothing was moved.";

/// Run `mrd move <OLD> <NEW> [flags]`.
#[allow(clippy::too_many_lines)]
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let parsed = Move::parse(args)?;
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(workspace::Base::Cwd(&cwd)).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;

    // The rooted lane on BOTH operands (address-grammar §4.6): OLD decides the
    // root; NEW may be ambient (inside that root) or name the same root.
    let old_lane = crate::rooted::enter(&parsed.old, "move", CONSEQUENCE)
        .map_err(|e| engine::json_refusal(parsed.format, &resolved.workspace, &e))?;
    let new_lane = crate::rooted::enter(parsed.new.trim_end_matches('/'), "move", CONSEQUENCE)
        .map_err(|e| engine::json_refusal(parsed.format, &resolved.workspace, &e))?;
    let (workspace, old, new, root_names, display_root) = match (old_lane, new_lane) {
        (Some((old_rel, r_old)), Some((new_rel, r_new))) => {
            if r_old.name != r_new.name {
                let mut e = wire::ErrorBody::new(wire::ErrorCode::BadPath);
                e.path = Some(wire::Path(parsed.new.clone()));
                e.message = Some(format!(
                    "cross_root: {} names root `{}` and {} names root `{}` — a move stays inside \
                     ONE root. {CONSEQUENCE}",
                    parsed.old, r_old.name, parsed.new, r_new.name
                ));
                return Err(engine::json_refusal(parsed.format, &r_old.workspace, &e));
            }
            let names = names_of(&r_old);
            let new = with_trailing_slash(new_rel, &parsed.new);
            (
                r_old.workspace,
                old_rel,
                new,
                names,
                Some(r_old.name.to_string()),
            )
        }
        (Some((old_rel, r_old)), None) => {
            let names = names_of(&r_old);
            (
                r_old.workspace,
                old_rel,
                parsed.new.clone(),
                names,
                Some(r_old.name.to_string()),
            )
        }
        (None, Some((_, r_new))) => {
            let mut e = wire::ErrorBody::new(wire::ErrorCode::BadPath);
            e.path = Some(wire::Path(parsed.new.clone()));
            e.message = Some(format!(
                "cross_root: {} names root `{}` while {} is an ambient path of the workspace you \
                 stand in — spell the root on both operands or on neither. {CONSEQUENCE}",
                parsed.new, r_new.name, parsed.old
            ));
            return Err(engine::json_refusal(parsed.format, &resolved.workspace, &e));
        }
        (None, None) => {
            let names = root_names_of(&resolved.workspace);
            (
                resolved.workspace.clone(),
                parsed.old.clone(),
                parsed.new.clone(),
                names,
                None,
            )
        }
    };

    // The §1 path law on both operands, before anything is read (exit 2).
    crate::path_law::admit(&workspace, old.trim_end_matches('/'), "move", CONSEQUENCE)?;
    crate::path_law::admit(&workspace, new.trim_end_matches('/'), "move", CONSEQUENCE)?;
    for prefix in &parsed.immutable {
        crate::path_law::admit(
            &workspace,
            prefix.trim_end_matches('/'),
            "move",
            CONSEQUENCE,
        )?;
    }

    let args = RelocateArgs {
        old: wire::Path(old),
        new: wire::Path(new),
        immutable: parsed.immutable.clone(),
        root_names,
        dry: parsed.dry,
    };
    let root = fs::WorkspaceRoot(workspace.clone());
    let outcome =
        relocate(&root, &args).map_err(|e| engine::json_refusal(parsed.format, &workspace, &e))?;

    let refused = (!outcome.plan.ambiguous.is_empty()).then(|| ambiguity_refusal(&outcome.plan));
    let unpredicted = outcome
        .read_back
        .filter(|read| *read != outcome.plan.after)
        .map(|read| {
            format!(
                "the read-back disagrees with the plan: planned {} resolved / {} dangling, disk \
                 reads {} resolved / {} dangling — the bytes landed; the plan was wrong",
                outcome.plan.after.resolved,
                outcome.plan.after.dangling,
                read.resolved,
                read.dangling
            )
        });

    match parsed.format {
        Format::Json => {
            let mut frame = json!({
                "workspace": workspace.display().to_string(),
                "move": frame_of(&outcome, display_root.as_deref()),
            });
            if let Some(error) = &refused {
                frame["error"] = engine_error_value(error);
            }
            if let Some(text) = &unpredicted {
                frame["unpredicted"] = json!(text);
            }
            println!("{}", serde_json::to_string_pretty(&frame).expect("json"));
        }
        Format::Human => print_human(&outcome, display_root.as_deref()),
    }

    if let Some(error) = refused {
        return Err(Fail::findings(engine::refusal_text(&error)));
    }
    if let Some(text) = unpredicted {
        return Err(Fail::findings(text));
    }
    Ok(())
}

/// The mount's canonical name and the alias it was reached by, for the class-3
/// rooted strings the plan rewrites.
fn names_of(rooted: &crate::rooted::RootedRef) -> Vec<String> {
    let mut names = vec![rooted.name.to_string()];
    if let Some(alias) = &rooted.alias {
        names.push(alias.to_string());
    }
    names
}

/// The names the ambient workspace answers to, read off this machine's mount
/// table; empty when the tree is bound under no name (then no rooted string
/// can name it, and class 3 has nothing to rewrite).
fn root_names_of(workspace: &Path) -> Vec<String> {
    let env = config::Env::from_process();
    let Ok(resolution) = config::resolve(&env) else {
        return Vec::new();
    };
    let Ok(table) = resolution.bind(&env) else {
        return Vec::new();
    };
    let Some(mount) = table.by_path(workspace) else {
        return Vec::new();
    };
    let mut names = vec![mount.name().to_owned()];
    if let Some(alias) = mount.alias() {
        names.push(alias.to_owned());
    }
    names
}

/// The rel half of a rooted NEW keeps the caller's into-form marker.
fn with_trailing_slash(rel: String, spelled: &str) -> String {
    if spelled.ends_with('/') && !rel.ends_with('/') {
        format!("{rel}/")
    } else {
        rel
    }
}

fn engine_error_value(error: &wire::ErrorBody) -> Value {
    let mut frame = json!({ "error": serde_json::to_value(error).expect("json") });
    wire_serve::rev::project_response(&mut frame);
    frame
        .as_object_mut()
        .and_then(|obj| obj.remove("error"))
        .unwrap_or(Value::Null)
}

fn spell(root: Option<&str>, rel: &str) -> String {
    match root {
        Some(root) => format!("{root}:{rel}"),
        None => rel.to_owned(),
    }
}

fn frame_of(outcome: &RelocateOutcome, root: Option<&str>) -> Value {
    let plan = &outcome.plan;
    let census =
        |c: &query::relocate::LinkCensus| json!({"resolved": c.resolved, "dangling": c.dangling});
    json!({
        "dry": outcome.dry,
        "applied": outcome.applied,
        "old": spell(root, &outcome.old),
        "new": spell(root, &outcome.new),
        "kind": if outcome.is_dir { "directory" } else { "file" },
        "renames": plan.renames.iter().map(|(from, to)| json!({"from": from, "to": to})).collect::<Vec<_>>(),
        "rewrites": plan.rewrites.iter().map(|f| json!({
            "path": f.path,
            "wikilinks": f.count(query::relocate::RefKind::Wikilink),
            "embeds": f.count(query::relocate::RefKind::Embed),
            "frontmatter": f.count(query::relocate::RefKind::Frontmatter),
            "rooted": f.count(query::relocate::RefKind::Rooted),
            "lock_rows": f.lock_rows(),
        })).collect::<Vec<_>>(),
        "immutable": plan.immutable.iter().map(|s| json!({
            "path": s.path, "line": s.line, "kind": s.kind.word(), "old": s.old, "new": s.new,
        })).collect::<Vec<_>>(),
        "ambiguous": plan.ambiguous.iter().map(|a| json!({
            "source": a.source, "linkpath": a.linkpath, "candidates": a.candidates,
        })).collect::<Vec<_>>(),
        "lock_unreadable": plan.lock_unreadable,
        "counts": {
            "files_rewritten": plan.rewrites.len(),
            "links_rewritten": plan.links_rewritten(),
            "lock_rows_rewritten": plan.lock_rows_rewritten(),
            "immutable_skips": plan.immutable.len(),
            "moved_outside_domain": outcome.moved_outside_domain,
        },
        "links": {
            "before": census(&plan.before),
            "after": census(&plan.after),
            "read_back": outcome.read_back.as_ref().map(census),
        },
    })
}

fn print_human(outcome: &RelocateOutcome, root: Option<&str>) {
    let plan = &outcome.plan;
    let kind = if outcome.is_dir { "directory" } else { "file" };
    let head = if outcome.dry {
        "dry run: would move"
    } else {
        "move"
    };
    println!(
        "{head} {} → {} ({kind}, {} corpus page(s), {} other file(s))",
        spell(root, &outcome.old),
        spell(root, &outcome.new),
        plan.renames.len(),
        outcome.moved_outside_domain
    );
    println!(
        "  rewrites: {} file(s) · {} link(s) · {} lock row(s)",
        plan.rewrites.len(),
        plan.links_rewritten(),
        plan.lock_rows_rewritten()
    );
    for file in &plan.rewrites {
        let mut parts = Vec::new();
        for kind in [
            query::relocate::RefKind::Wikilink,
            query::relocate::RefKind::Embed,
            query::relocate::RefKind::Frontmatter,
            query::relocate::RefKind::Rooted,
        ] {
            let n = file.count(kind);
            if n > 0 {
                parts.push(format!("{} {n}", kind.word()));
            }
        }
        if file.lock_rows() > 0 {
            parts.push(format!("lock {}", file.lock_rows()));
        }
        println!("    {}  {}", file.path, parts.join(" · "));
    }
    if !plan.immutable.is_empty() {
        println!("  immutable skips: {}", plan.immutable.len());
        for skip in &plan.immutable {
            println!(
                "    {}:{}  {}  {} → {}",
                skip.path,
                skip.line,
                skip.kind.word(),
                skip.old,
                skip.new
            );
        }
    }
    if !plan.lock_unreadable.is_empty() {
        println!(
            "  lock blocks not read (corrupt): {}",
            plan.lock_unreadable.join(", ")
        );
    }
    if !plan.ambiguous.is_empty() {
        println!(
            "  refused — ambiguous after the move: {}",
            plan.ambiguous.len()
        );
        for a in &plan.ambiguous {
            println!(
                "    {}  [[{}]] → {}",
                a.source,
                a.linkpath,
                a.candidates.join(" | ")
            );
        }
    }
    let read_back = outcome
        .read_back
        .as_ref()
        .map(|read| format!(" · read back {} / {}", read.resolved, read.dangling))
        .unwrap_or_default();
    println!(
        "  links: before {} resolved / {} dangling · after (planned) {} / {}{read_back}",
        plan.before.resolved, plan.before.dangling, plan.after.resolved, plan.after.dangling
    );
    if outcome.applied {
        println!("moved.");
    } else if outcome.dry {
        println!("dry run: nothing written.");
    } else {
        println!("refused: nothing written.");
    }
}

#[derive(Debug)]
struct Move {
    old: String,
    new: String,
    immutable: Vec<String>,
    dry: bool,
    format: Format,
}

impl Move {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut positionals: Vec<String> = Vec::new();
        let mut immutable = Vec::new();
        let mut dry = false;
        let mut json = false;
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--json" => json = true,
                "--dry" => dry = true,
                "--immutable" => {
                    let value = it
                        .next()
                        .ok_or_else(|| Fail::tool("--immutable needs a PREFIX".to_owned()))?;
                    if value.trim_end_matches('/').is_empty() {
                        return Err(Fail::tool(
                            "--immutable needs a workspace-relative PREFIX, not the root itself"
                                .to_owned(),
                        ));
                    }
                    immutable.push(value.clone());
                }
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value if positionals.len() < 2 => positionals.push(value.to_owned()),
                value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
            }
        }
        let [old, new] = positionals.as_slice() else {
            return Err(Fail::tool(
                "move needs an OLD and a NEW path — `mrd move <OLD> <NEW>`; NEW ending in `/` \
                 lands OLD under it"
                    .to_owned(),
            ));
        };
        if old.trim_end_matches('/').is_empty() || new.trim_end_matches('/').is_empty() {
            return Err(Fail::tool(
                "move needs workspace-relative paths — the root itself cannot move".to_owned(),
            ));
        }
        Ok(Move {
            old: old.clone(),
            new: new.clone(),
            immutable,
            dry,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Move;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn move_needs_two_operands() {
        let err = Move::parse(&args(&["a/x.md"])).expect_err("one operand refuses");
        assert!(format!("{err:?}").contains("OLD and a NEW"), "{err:?}");
    }

    #[test]
    fn move_parse_carries_the_flags() {
        let m = Move::parse(&args(&[
            "a/x.md",
            "b/",
            "--dry",
            "--immutable",
            "sources/",
            "--immutable",
            "logs",
            "--json",
        ]))
        .expect("parses");
        assert_eq!(m.old, "a/x.md");
        assert_eq!(m.new, "b/");
        assert_eq!(m.immutable, vec!["sources/", "logs"]);
        assert!(m.dry);
    }

    #[test]
    fn a_third_operand_is_a_bad_invocation() {
        let err = Move::parse(&args(&["a", "b", "c"])).expect_err("three operands refuse");
        assert!(
            format!("{err:?}").contains("unexpected argument"),
            "{err:?}"
        );
    }
}
