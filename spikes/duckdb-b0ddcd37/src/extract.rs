//! Cheap single-pass markdown fact extractor.
//! NOT the subject of measurement — reports extract time separately from DB time.
//! Zero non-duckdb deps: hand-rolled walk, line-based frontmatter, byte-scan links.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Deterministic 64-bit content hash. `DefaultHasher::new()` uses fixed keys
/// (SipHash-1-3 with keys 0,0), so revs/roots are stable across runs/processes.
pub fn hash64(bytes: &[u8]) -> i64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish() as i64
}

pub struct FileRow {
    pub path: String,
    pub norm_key: String,
    pub file_rev: i64,
    pub bytes: i64,
}
pub struct NodeRow {
    pub path: String,
    pub kind: String,
    pub span_start: i64,
    pub span_end: i64,
    pub hpath: String,
    pub node_rev: i64,
}
pub struct LinkRow {
    pub src_path: String,
    pub target: String,
    pub resolved: bool,
}
pub struct FmRow {
    pub path: String,
    pub key: String,
    pub value: String,
}

#[derive(Default)]
pub struct Facts {
    pub files: Vec<FileRow>,
    pub nodes: Vec<NodeRow>,
    pub links: Vec<LinkRow>,
    pub frontmatter: Vec<FmRow>,
}

/// last path component, lowercased, `.md` stripped — Obsidian-ish resolution key.
pub fn norm_key(target: &str) -> String {
    let last = target.rsplit(['/', '\\']).next().unwrap_or(target);
    let stem = last.strip_suffix(".md").unwrap_or(last);
    stem.trim().to_ascii_lowercase()
}

/// Recursively collect `*.md` paths under `root`, skipping dot-directories.
pub fn walk_md(root: &Path, out: &mut Vec<PathBuf>) {
    let rd = match std::fs::read_dir(root) {
        Ok(r) => r,
        Err(_) => return,
    };
    for ent in rd.flatten() {
        let name = ent.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue; // dot-dirs and dot-files excluded (md-only ruling default)
        }
        let ft = match ent.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let p = ent.path();
        if ft.is_dir() {
            walk_md(&p, out);
        } else if ft.is_file() && name.ends_with(".md") {
            out.push(p);
        }
    }
}

/// Two-phase extract: phase 1 lists paths (builds resolution set from basenames,
/// no reads); phase 2 reads each file once and pulls facts.
/// Returns (facts, files_read, bytes_read).
pub fn extract_corpus(roots: &[PathBuf]) -> (Facts, usize, u64) {
    let mut paths = Vec::new();
    for r in roots {
        walk_md(r, &mut paths);
    }
    paths.sort();

    // phase 1: resolution set of known normalized page keys
    let mut known: HashSet<String> = HashSet::with_capacity(paths.len() * 2);
    for p in &paths {
        if let Some(stem) = p.file_stem() {
            known.insert(stem.to_string_lossy().to_ascii_lowercase());
        }
    }

    // phase 2: read + extract
    let mut facts = Facts::default();
    let mut bytes_read: u64 = 0;
    for p in &paths {
        let content = match std::fs::read(p) {
            Ok(c) => c,
            Err(_) => continue,
        };
        bytes_read += content.len() as u64;
        let path_str = p.to_string_lossy().into_owned();
        let nk = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        facts.files.push(FileRow {
            path: path_str.clone(),
            norm_key: nk,
            file_rev: hash64(&content),
            bytes: content.len() as i64,
        });
        // treat as UTF-8 lossily for text scanning; bytes hash uses raw bytes
        let text = String::from_utf8_lossy(&content);
        extract_one(&path_str, &text, &known, &mut facts);
    }
    let n = facts.files.len();
    (facts, n, bytes_read)
}

/// Extract nodes, links, frontmatter for a single file into `facts`.
pub fn extract_one(path: &str, text: &str, known: &HashSet<String>, facts: &mut Facts) {
    // ---- frontmatter (leading --- ... --- block) ----
    let fm_end = parse_frontmatter(path, text, facts);

    // ---- headings / section nodes ----
    // collect (level, title, start_byte) then resolve spans + hpath
    let mut heads: Vec<(u8, String, usize)> = Vec::new();
    let mut off = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start_matches([' ', '\t']);
        if let Some(rest) = leading_hashes(trimmed) {
            let level = rest.0;
            let title = rest.1.trim().to_string();
            // heading offset within the whole file
            let start = off + (line.len() - trimmed.len());
            heads.push((level, title, start));
        }
        off += line.len();
    }
    let file_len = text.len();
    let mut stack: Vec<(u8, String)> = Vec::new();
    for i in 0..heads.len() {
        let (level, ref title, start) = heads[i];
        let end = if i + 1 < heads.len() {
            heads[i + 1].2
        } else {
            file_len
        };
        while let Some(top) = stack.last() {
            if top.0 >= level {
                stack.pop();
            } else {
                break;
            }
        }
        stack.push((level, title.clone()));
        let hpath = stack
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join(" > ");
        let section = &text[start..end.min(text.len())];
        facts.nodes.push(NodeRow {
            path: path.to_string(),
            kind: format!("h{level}"),
            span_start: start as i64,
            span_end: end as i64,
            hpath,
            node_rev: hash64(section.as_bytes()),
        });
    }

    // ---- links (wikilinks + internal markdown links), skip frontmatter block ----
    let body = &text[fm_end.min(text.len())..];
    extract_links(path, body, known, facts);
}

/// Returns `(level, title_str)` if the trimmed line is an ATX heading.
fn leading_hashes(s: &str) -> Option<(u8, &str)> {
    let mut n = 0u8;
    for b in s.bytes() {
        if b == b'#' {
            n += 1;
            if n > 6 {
                return None;
            }
        } else {
            break;
        }
    }
    if n == 0 {
        return None;
    }
    let rest = &s[n as usize..];
    // must be "# " (space/tab after hashes) to be a heading
    if rest.starts_with([' ', '\t']) {
        Some((n, rest))
    } else if rest.is_empty() || rest.starts_with('\n') {
        Some((n, ""))
    } else {
        None
    }
}

/// Cheap line-based frontmatter parse. Returns byte offset where FM block ends
/// (0 if none) so link extraction can skip it.
fn parse_frontmatter(path: &str, text: &str, facts: &mut Facts) -> usize {
    // must start with '---' on the first line
    let first = text.split_inclusive('\n').next().unwrap_or("");
    if first.trim_end() != "---" {
        return 0;
    }
    let mut off = first.len();
    let mut last_key: Option<String> = None;
    for line in text[off..].split_inclusive('\n') {
        let raw = line;
        let l = line.trim_end();
        off += raw.len();
        if l.trim() == "---" || l.trim() == "..." {
            return off; // end of FM block (offset just past the closing ---)
        }
        let lt = l.trim();
        if lt.is_empty() {
            continue;
        }
        if let Some(item) = lt.strip_prefix("- ") {
            if let Some(k) = &last_key {
                facts.frontmatter.push(FmRow {
                    path: path.to_string(),
                    key: k.clone(),
                    value: clean_val(item),
                });
            }
            continue;
        }
        if let Some(ci) = lt.find(':') {
            let key = lt[..ci].trim().to_string();
            let rest = lt[ci + 1..].trim();
            last_key = Some(key.clone());
            if rest.is_empty() {
                continue; // list or nested follows
            }
            // inline array [a, b, c]
            if rest.starts_with('[') && rest.ends_with(']') {
                let inner = &rest[1..rest.len() - 1];
                for part in inner.split(',') {
                    let v = clean_val(part);
                    if !v.is_empty() {
                        facts.frontmatter.push(FmRow {
                            path: path.to_string(),
                            key: key.clone(),
                            value: v,
                        });
                    }
                }
            } else {
                facts.frontmatter.push(FmRow {
                    path: path.to_string(),
                    key,
                    value: clean_val(rest),
                });
            }
        }
    }
    off
}

/// Strip surrounding quotes / wikilink brackets / whitespace from an FM value.
fn clean_val(s: &str) -> String {
    let mut v = s.trim();
    v = v.trim_matches(|c| c == '"' || c == '\'');
    v = v.trim();
    // "[[Foo]]" -> "Foo"
    if let Some(inner) = v.strip_prefix("[[").and_then(|x| x.strip_suffix("]]")) {
        v = inner.trim();
    }
    v.to_string()
}

/// Scan `body` for `[[wikilinks]]` and internal `[md](links)`; push LinkRows.
fn extract_links(path: &str, body: &str, known: &HashSet<String>, facts: &mut Facts) {
    let b = body.as_bytes();
    let mut i = 0usize;
    while i + 1 < b.len() {
        // wikilink [[ ... ]]
        if b[i] == b'[' && b[i + 1] == b'[' {
            if let Some(close) = find2(b, i + 2, b']') {
                let inner = &body[i + 2..close];
                if let Some(t) = wikilink_target(inner) {
                    let nk = norm_key(&t);
                    if !nk.is_empty() {
                        facts.links.push(LinkRow {
                            src_path: path.to_string(),
                            target: nk.clone(),
                            resolved: known.contains(&nk),
                        });
                    }
                }
                i = close + 2;
                continue;
            }
        }
        // markdown link ](url)
        if b[i] == b']' && i + 1 < b.len() && b[i + 1] == b'(' {
            if let Some(close) = memchr(b, i + 2, b')') {
                let url = &body[i + 2..close];
                if let Some(t) = md_internal_target(url) {
                    let nk = norm_key(&t);
                    if !nk.is_empty() {
                        facts.links.push(LinkRow {
                            src_path: path.to_string(),
                            target: nk.clone(),
                            resolved: known.contains(&nk),
                        });
                    }
                }
                i = close + 1;
                continue;
            }
        }
        i += 1;
    }
}

/// find `]]` starting at `from`; returns index of first `]` of the pair.
fn find2(b: &[u8], from: usize, ch: u8) -> Option<usize> {
    let mut i = from;
    while i + 1 < b.len() {
        if b[i] == ch && b[i + 1] == ch {
            return Some(i);
        }
        if b[i] == b'\n' {
            return None; // wikilinks don't span lines
        }
        i += 1;
    }
    None
}

fn memchr(b: &[u8], from: usize, ch: u8) -> Option<usize> {
    let mut i = from;
    while i < b.len() {
        if b[i] == ch {
            return Some(i);
        }
        if b[i] == b'\n' {
            return None;
        }
        i += 1;
    }
    None
}

fn wikilink_target(inner: &str) -> Option<String> {
    let s = inner.trim();
    if s.is_empty() {
        return None;
    }
    // strip alias |, section #, block ^
    let s = s.split('|').next().unwrap_or(s);
    let s = s.split('#').next().unwrap_or(s);
    let s = s.split('^').next().unwrap_or(s);
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

fn md_internal_target(url: &str) -> Option<String> {
    let u = url.trim();
    if u.is_empty() {
        return None;
    }
    // external / anchor / mailto → not an internal page link
    if u.starts_with("http://")
        || u.starts_with("https://")
        || u.starts_with('#')
        || u.starts_with("mailto:")
        || u.contains("://")
    {
        return None;
    }
    // only treat .md targets as internal page links (cheap heuristic)
    let base = u.split('#').next().unwrap_or(u);
    if base.ends_with(".md") {
        Some(base.to_string())
    } else {
        None
    }
}
