//! Human rendering of wire response frames — presentation only, one screen
//! line per wire fact. The raw frame is authoritative (`--json` emits it
//! verbatim); nothing here re-interprets wire semantics.

use wire::{Response, ResponseBody, ResponsePayload, SecRef, TocNode};

/// Render one response (+ any Notification frames) for people. Success
/// prose goes to stdout; error prose goes to stderr (the exit code already
/// says which happened).
pub(crate) fn human(response: &Response, notifications: &[&str]) {
    match &response.payload {
        ResponsePayload::Body { body } => {
            body_human(body);
            if !notifications.is_empty() {
                // sub replay: deltas are data — pass the frames through raw.
                for n in notifications {
                    println!("{n}");
                }
            }
        }
        ResponsePayload::Error { error } => {
            eprintln!(
                "mrd: error: {} (recovery: {})",
                flat(&error.code),
                flat(&error.recovery)
            );
            if let Some(msg) = &error.message {
                eprintln!("  {msg}");
            }
            // The code-specific extras, one per line, straight off the frame.
            if let Ok(serde_json::Value::Object(map)) = serde_json::to_value(error) {
                for (k, v) in &map {
                    if !matches!(k.as_str(), "code" | "recovery" | "message") {
                        eprintln!("  {k}: {v}");
                    }
                }
            }
        }
    }
}

/// A flat lowercase wire enum (`ErrorCode`, `Recovery`) as its wire string.
fn flat<T: serde::Serialize>(e: &T) -> String {
    serde_json::to_value(e)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "?".into())
}

fn body_human(body: &ResponseBody) {
    match body {
        ResponseBody::Hello {
            proto,
            server,
            caps,
            root,
        } => {
            println!("server: {server} (proto {proto})");
            println!("caps:   {}", caps.join(" "));
            if let Some(root) = root {
                println!("root:   {}", root.0);
            }
        }
        ResponseBody::Toc {
            path,
            file_rev,
            root,
            nodes,
        } => {
            println!("{} file_rev={} root={}", path.0, file_rev.0, root.0);
            for node in nodes {
                println!("{}", toc_row(node));
            }
        }
        ResponseBody::Nodes { path, nodes } => {
            println!("{} ({} nodes)", path.0, nodes.len());
            for n in nodes {
                let kind = flat(&n.kind);
                let rev = n
                    .node_rev
                    .as_ref()
                    .map(|r| format!(" rev={}", r.0))
                    .unwrap_or_default();
                let hpath = n
                    .hpath
                    .as_ref()
                    .map(|h| format!(" {}", hpath_text(h)))
                    .unwrap_or_default();
                println!(
                    "  {kind} [{},{}){rev}{hpath} {:?}",
                    n.span.0, n.span.1, n.text_prefix_16b
                );
            }
        }
        ResponseBody::Cat { content, .. } => {
            // The section bytes, nothing else — pipe-friendly; span/rev ride
            // `--json`.
            print!("{content}");
        }
        ResponseBody::Resolve {
            dest,
            span,
            content,
        } => {
            println!("{} [{},{})", dest.0, span.0, span.1);
            if let Some(content) = content {
                print!("{content}");
            }
        }
        ResponseBody::Splice {
            armed,
            receipt,
            root_before,
            root_after,
            seq,
            dry,
            verdicts,
        } => splice_human(
            armed,
            receipt.as_ref(),
            root_before,
            root_after.as_ref(),
            *seq,
            *dry,
            verdicts,
        ),
        ResponseBody::Root { root, seq } => {
            println!("root: {}", root.0);
            println!("seq:  {seq}");
        }
        ResponseBody::Diff { batches } => diff_human(batches),
        ResponseBody::Links {
            as_of_root,
            live_root,
            changes_seq,
            files,
        } => links_human(as_of_root, live_root, *changes_seq, files),
    }
}

fn splice_human(
    armed: &wire::Armed,
    receipt: Option<&wire::ReceiptFact>,
    root_before: &wire::Root,
    root_after: Option<&wire::Root>,
    seq: Option<u64>,
    dry: Option<bool>,
    verdicts: &[wire::Verdict],
) {
    let mode = if dry.unwrap_or(false) {
        "would splice (dry)"
    } else {
        "spliced"
    };
    println!(
        "{mode} {} ({} edit{})",
        armed.path.0,
        armed.edits.len(),
        if armed.edits.len() == 1 { "" } else { "s" }
    );
    for e in &armed.edits {
        println!(
            "  {}: {} -> {} span_after=[{},{})",
            sec_ref_text(&e.target),
            e.node_rev_before.0,
            e.node_rev_after.0,
            e.span_after.0,
            e.span_after.1
        );
    }
    if let Some(r) = receipt {
        println!(
            "receipt: {} ^{} rev={} span_after=[{},{})",
            r.path.0, r.anchor, r.node_rev.0, r.span_after.0, r.span_after.1
        );
    }
    match root_after {
        Some(after) => println!("root: {} -> {}", root_before.0, after.0),
        None => println!("root: {} -> (dry, unchanged)", root_before.0),
    }
    if let Some(seq) = seq {
        println!("seq: {seq}");
    }
    for v in verdicts {
        println!(
            "verdict: [{}] {} {} — {}",
            flat(&v.severity),
            v.rule,
            v.path.0,
            v.message
        );
    }
}

fn diff_human(batches: &[wire::DeltaFrame]) {
    println!("{} delta batch(es)", batches.len());
    for b in batches {
        let d = &b.delta;
        let actor = d
            .actor
            .as_ref()
            .map(|a| format!(" actor={a}"))
            .unwrap_or_default();
        println!(
            "delta seq={}{} {} -> {}",
            d.seq, actor, d.root_before.0, d.root_after.0
        );
        for f in &d.files {
            println!("  {} {}", flat(&f.change), f.path.0);
            for n in &f.nodes {
                println!("    {} {}", flat(&n.change), sec_ref_text(&n.target));
            }
        }
    }
}

fn links_human(
    as_of_root: &wire::Root,
    live_root: &wire::Root,
    changes_seq: u64,
    files: &std::collections::BTreeMap<String, wire::FileLinks>,
) {
    let tense = if as_of_root == live_root {
        "current"
    } else {
        "STALE"
    };
    println!(
        "as_of={} live={} changes_seq={changes_seq} ({tense})",
        as_of_root.0, live_root.0
    );
    for (path, links) in files {
        for (dest, count) in &links.resolved {
            println!("  {path} -> {dest} x{count}");
        }
        for (linkpath, count) in &links.unresolved {
            println!("  {path} -/-> {linkpath} x{count} (unresolved)");
        }
    }
}

fn toc_row(node: &TocNode) -> String {
    use std::fmt::Write as _;
    let mut row = format!("  {}", node.kind);
    if let Some(level) = node.level {
        let _ = write!(row, " L{level}");
    }
    if let Some(hpath) = &node.hpath {
        let _ = write!(row, " {}", hpath_text(hpath));
    }
    if let Some(anchor) = &node.anchor {
        let _ = write!(row, " ^{anchor}");
    }
    if let Some(keys) = &node.keys {
        let _ = write!(row, " keys={}", keys.join(","));
    }
    let _ = write!(
        row,
        " [{},{}) rev={}",
        node.span.0, node.span.1, node.node_rev.0
    );
    row
}

fn hpath_text(hpath: &[wire::HpathSeg]) -> String {
    hpath
        .iter()
        .map(|seg| match seg.n {
            Some(n) => format!("{}({n})", seg.h),
            None => seg.h.clone(),
        })
        .collect::<Vec<_>>()
        .join(" > ")
}

fn sec_ref_text(sec: &SecRef) -> String {
    match sec {
        SecRef::Hpath { hpath } => hpath_text(hpath),
        SecRef::Anchor { anchor } => format!("^{anchor}"),
        SecRef::FmKey { fm_key } => format!("fm:{fm_key}"),
    }
}
