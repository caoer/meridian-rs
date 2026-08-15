//! The sql premise binds ROW PROVENANCE, not the WHERE clause — the merged
//! plan §4.5's second law (lifted codex §4.2/§6.3, adjudicated), the planner
//! regions of card set-premises.
//!
//! The planner emits the dependency regions a query's result actually rests
//! on. A predicate on the rows' OWN file path bounds provenance to that
//! subtree, which contributes its fold. A query whose result certifies
//! corpus-wide facts — backlinks, dangling links, aggregates, any "no row
//! exists" reading — contributes the WORLD, whatever its WHERE clause says.
//! Unboundable provenance escalates to world: conservatively, honestly.
//!
//! The counterexample that forced this form, kept on the record (plan §4.5):
//! `backlink WHERE path = 'tasks/x.md'` is path-constrained and still WORLD —
//! a link written in `agents/y.md` changes the result while `tasks/` never
//! moves. `backlink.path` names the DESTINATION, not the row's own file; row
//! provenance of an inbound-edge fact is every file that could write the
//! edge, which is the corpus.
//!
//! # The relation catalog (documented against `view/src/schema.rs`, v8)
//!
//! ROW-BOUND — every row (and every row's absence) is a fact of its own
//! file, named by the relation's `path` column: `doc`, `frontmatter`,
//! `section`, `tag`, `frontmatter_tag`, `task`, `body`, `record` (a per-file
//! pivot: each row folds only its own file's frontmatter), `tag_all` (a
//! union of two row-bound relations sharing the `path` spelling).
//!
//! WORLD — the row is a corpus-computed fact: `link` (its `dest_path` /
//! `resolved` / `exclusion` columns re-resolve against the member set, so a
//! row changes when a FOREIGN file is born), `backlink`, `dangling` (a "no
//! file exists" reading), and the `.base` relations plus `_meridian_view`
//! (outside the fingerprint domain entirely — `base_fold` witness, §12.1
//! md-only floor — so no tree premise can bound them; world is the honest
//! conservative floor, stated). Any relation this catalog does not name —
//! `hist.*`, system tables, table functions — is unknown and escalates.
//!
//! # The recognizer, and why it is conservative
//!
//! Classification grants a bounded region ONLY to a shape it can prove: one
//! plain `SELECT` over one row-bound relation, no aggregation, whose WHERE
//! carries a top-level conjunct binding the relation's own `path` column to
//! a literal (`=`), a terminal-`%` prefix (`LIKE`), or a literal list
//! (`IN`). Everything else — joins, subqueries, CTEs, set ops, aggregate or
//! unknown functions, window frames, top-level `OR` — escalates to world.
//! Aggregates escalate BY LAW, not by implementation limit: §4.5's letter
//! names them world-class whatever the WHERE says; narrowing that would be a
//! law change, never an implementation choice here.
//!
//! Consistency law (plan §4.5): the emitted regions validate against the
//! TREE — [`SqlProvenance::scopes`] names the path nodes whose folds guard
//! the result, the same instrument as every other premise. No journal
//! appears in any signature of this module.

use std::collections::BTreeSet;

/// A query's dependency regions — what the premise plane folds and guards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqlProvenance {
    /// Path-bounded: the result rests on exactly these workspace-relative
    /// path nodes (a directory subtree or an exact member file per entry).
    Regions(BTreeSet<String>),
    /// Corpus-wide: the result rests on the whole tree — the root premise.
    World,
}

impl SqlProvenance {
    /// The scope paths to fold and guard, one premise per entry — the tree's
    /// spelling (root = the empty path). This is the seam the touch-set
    /// recording consumes: each scope folds via the resident tree
    /// (`fs::ResidentTree::fold_at`, or the §4.3.1 forest form), never via
    /// the journal.
    #[must_use]
    pub fn scopes(&self) -> Vec<String> {
        match self {
            SqlProvenance::World => vec![String::new()],
            SqlProvenance::Regions(regions) => regions.iter().cloned().collect(),
        }
    }
}

/// Classify one SQL statement's row provenance. Total: every input answers —
/// anything the recognizer cannot prove bounded is [`SqlProvenance::World`].
#[must_use]
pub fn classify(sql: &str) -> SqlProvenance {
    let Some(mut tokens) = tokenize(sql) else {
        return SqlProvenance::World;
    };
    // A trailing statement terminator is noise, not shape; an INTERIOR `;`
    // (a second statement) leaves two `select`s and escalates below.
    while tokens.last() == Some(&Tok::Punct(';')) {
        tokens.pop();
    }
    match bounded_regions(&tokens) {
        Some(regions) => SqlProvenance::Regions(regions),
        None => SqlProvenance::World,
    }
}

/// Relations whose rows are facts of their own file, all spelling their own
/// path column `path` (catalog rationale: module docs).
const ROW_BOUND: [&str; 9] = [
    "doc",
    "frontmatter",
    "section",
    "tag",
    "frontmatter_tag",
    "task",
    "body",
    "record",
    "tag_all",
];

/// Words that end a FROM clause or a WHERE clause legally.
const CLAUSE_TAILS: [&str; 3] = ["order", "limit", "offset"];

/// Words anywhere in the statement that escalate on sight: multi-relation
/// shapes, set ops, aggregation frames, CTEs.
const WORLD_WORDS: [&str; 12] = [
    "with",
    "union",
    "intersect",
    "except",
    "group",
    "having",
    "over",
    "window",
    "join",
    "natural",
    "lateral",
    "qualify",
];

/// Words legally followed by `(` without being a function call, plus the
/// scalar functions the recognizer trusts to keep row provenance (a scalar
/// of a row's own columns). ANY other word before `(` — aggregate, table
/// function, macro, unknown — escalates: soundness comes from the whitelist
/// direction, so an unlisted aggregate can never slip through as bounded.
const ALLOWED_BEFORE_PAREN: [&str; 36] = [
    // structural words
    "in",
    "and",
    "or",
    "not",
    "on",
    "where",
    "select",
    "when",
    "then",
    "else",
    "between",
    "like",
    "is",
    "as",
    "by",
    "case",
    // row-scalar functions
    "lower",
    "upper",
    "trim",
    "ltrim",
    "rtrim",
    "length",
    "len",
    "substr",
    "substring",
    "replace",
    "concat",
    "coalesce",
    "nullif",
    "abs",
    "round",
    "cast",
    "try_cast",
    "contains",
    "starts_with",
    "ends_with",
];

/// One token: a lowercased bare word, a string literal's payload, or a
/// single punctuation character. Quoted identifiers deliberately surface as
/// punctuation — the recognizer then fails to prove the shape and escalates.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Word(String),
    Str(String),
    Punct(char),
}

/// Lex the statement. `None` on an unterminated string or comment — the
/// caller escalates to world.
fn tokenize(sql: &str) -> Option<Vec<Tok>> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out: Vec<Tok> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '-' && chars.get(i + 1) == Some(&'-') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            let mut end = None;
            let mut j = i + 2;
            while j + 1 < chars.len() {
                if chars[j] == '*' && chars[j + 1] == '/' {
                    end = Some(j + 2);
                    break;
                }
                j += 1;
            }
            i = end?;
        } else if c == '\'' {
            let mut payload = String::new();
            let mut j = i + 1;
            loop {
                match chars.get(j) {
                    None => return None,
                    Some('\'') if chars.get(j + 1) == Some(&'\'') => {
                        payload.push('\'');
                        j += 2;
                    }
                    Some('\'') => {
                        j += 1;
                        break;
                    }
                    Some(&ch) => {
                        payload.push(ch);
                        j += 1;
                    }
                }
            }
            out.push(Tok::Str(payload));
            i = j;
        } else if c == '"' {
            // Quoted identifier: skip the payload, surface as punctuation so
            // no bounded shape can be proven through it.
            let mut j = i + 1;
            loop {
                match chars.get(j) {
                    None => return None,
                    Some('"') if chars.get(j + 1) == Some(&'"') => j += 2,
                    Some('"') => {
                        j += 1;
                        break;
                    }
                    Some(_) => j += 1,
                }
            }
            out.push(Tok::Punct('"'));
            i = j;
        } else if c.is_ascii_alphabetic() || c == '_' {
            let mut word = String::new();
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '$')
            {
                word.push(chars[i].to_ascii_lowercase());
                i += 1;
            }
            out.push(Tok::Word(word));
        } else if c.is_ascii_digit() {
            let mut word = String::new();
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                word.push(chars[i]);
                i += 1;
            }
            out.push(Tok::Word(word));
        } else {
            out.push(Tok::Punct(c));
            i += 1;
        }
    }
    Some(out)
}

/// The whole recognizer: prove the one bounded shape or answer `None`.
fn bounded_regions(tokens: &[Tok]) -> Option<BTreeSet<String>> {
    let is_word = |t: &Tok, s: &str| matches!(t, Tok::Word(w) if w == s);

    // One plain SELECT; no escalating word anywhere; no un-whitelisted call.
    if tokens.iter().filter(|t| is_word(t, "select")).count() != 1 {
        return None;
    }
    for t in tokens {
        if let Tok::Word(w) = t
            && WORLD_WORDS.contains(&w.as_str())
        {
            return None;
        }
    }
    for pair in tokens.windows(2) {
        if let (Tok::Word(w), Tok::Punct('(')) = (&pair[0], &pair[1])
            && !ALLOWED_BEFORE_PAREN.contains(&w.as_str())
        {
            return None;
        }
    }

    // FROM exactly one row-bound relation, with an optional alias.
    let from = tokens.iter().position(|t| is_word(t, "from"))?;
    if tokens.iter().skip(from + 1).any(|t| is_word(t, "from")) {
        return None;
    }
    let Some(Tok::Word(relation)) = tokens.get(from + 1) else {
        return None;
    };
    if !ROW_BOUND.contains(&relation.as_str()) {
        return None;
    }
    let mut alias = relation.clone();
    let mut after = from + 2;
    match tokens.get(after) {
        Some(Tok::Word(w)) if w == "as" => {
            let Some(Tok::Word(a)) = tokens.get(after + 1) else {
                return None;
            };
            alias.clone_from(a);
            after += 2;
        }
        Some(Tok::Word(w)) if w != "where" && !CLAUSE_TAILS.contains(&w.as_str()) => {
            alias.clone_from(w);
            after += 1;
        }
        _ => {}
    }
    // After the relation only WHERE or a tail clause may follow (a comma
    // would be a second relation; a missing WHERE reads every row: world).
    match tokens.get(after) {
        Some(Tok::Word(w)) if w == "where" || CLAUSE_TAILS.contains(&w.as_str()) => {}
        None | Some(_) => return None,
    }

    // The WHERE region, split into top-level conjuncts.
    let where_at = tokens.iter().position(|t| is_word(t, "where"))?;
    let mut end = tokens.len();
    let mut depth = 0usize;
    for (i, t) in tokens.iter().enumerate().skip(where_at + 1) {
        match t {
            Tok::Punct('(') => depth += 1,
            Tok::Punct(')') => depth = depth.checked_sub(1)?,
            Tok::Word(w) if depth == 0 && CLAUSE_TAILS.contains(&w.as_str()) => {
                end = i;
                break;
            }
            // A top-level OR makes the whole WHERE a disjunction — no
            // conjunct is a sound bound on its own.
            Tok::Word(w) if depth == 0 && w == "or" => return None,
            Tok::Word(_) | Tok::Str(_) | Tok::Punct(_) => {}
        }
    }
    let clause = &tokens[where_at + 1..end];
    let mut start = 0;
    let mut depth = 0usize;
    for i in 0..=clause.len() {
        let split = match clause.get(i) {
            None => true,
            Some(Tok::Punct('(')) => {
                depth += 1;
                false
            }
            Some(Tok::Punct(')')) => {
                depth = depth.checked_sub(1)?;
                false
            }
            Some(t) => depth == 0 && is_word(t, "and"),
        };
        if split {
            if let Some(regions) = path_conjunct(&clause[start..i], relation, &alias) {
                return Some(regions);
            }
            start = i + 1;
        }
    }
    None
}

/// Try to read one conjunct as a bound on the relation's own `path` column:
/// `[qual.]path = 'lit'` · `[qual.]path LIKE 'prefix%'` · `[qual.]path IN
/// ('a', …)`. Answers the granted region set, or `None` when this conjunct
/// proves nothing (it then merely narrows and the scan continues).
fn path_conjunct(conjunct: &[Tok], relation: &str, alias: &str) -> Option<BTreeSet<String>> {
    let is_word = |t: &Tok, s: &str| matches!(t, Tok::Word(w) if w == s);
    // Peel the optional qualifier, which must name the relation or its alias.
    let rest = match conjunct {
        [Tok::Word(q), Tok::Punct('.'), rest @ ..] if q == relation || q == alias => rest,
        rest => rest,
    };
    let ops = match rest {
        [first, rest @ ..] if is_word(first, "path") => rest,
        _ => return None,
    };
    match ops {
        [Tok::Punct('='), Tok::Str(lit)] if !lit.is_empty() => Some(BTreeSet::from([lit.clone()])),
        [like, Tok::Str(pattern)] if is_word(like, "like") => {
            like_region(pattern).map(|r| BTreeSet::from([r]))
        }
        [inn, Tok::Punct('('), members @ .., Tok::Punct(')')] if is_word(inn, "in") => {
            let mut regions = BTreeSet::new();
            for (i, t) in members.iter().enumerate() {
                match t {
                    Tok::Str(lit) if i % 2 == 0 && !lit.is_empty() => {
                        regions.insert(lit.clone());
                    }
                    Tok::Punct(',') if i % 2 == 1 => {}
                    _ => return None,
                }
            }
            if members.len() % 2 == 1 {
                Some(regions)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// The subtree a `LIKE` pattern provably bounds: exactly one `%`, terminal,
/// no `_`, and a `/` inside the literal prefix — the region is the deepest
/// directory prefix (`tasks/%` → `tasks`; `tasks/x%` → `tasks`). Anything
/// else proves nothing.
fn like_region(pattern: &str) -> Option<String> {
    if pattern.contains('_') || !pattern.ends_with('%') {
        return None;
    }
    let prefix = &pattern[..pattern.len() - 1];
    if prefix.contains('%') || prefix.is_empty() {
        return None;
    }
    let slash = prefix.rfind('/')?;
    if slash == 0 {
        return None;
    }
    Some(prefix[..slash].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regions(entries: &[&str]) -> SqlProvenance {
        SqlProvenance::Regions(entries.iter().map(|s| (*s).to_owned()).collect())
    }

    /// The §7 sql row's bounded arm: a predicate on the rows' own file path
    /// bounds provenance to that subtree — `=`, terminal-`%` `LIKE`, and
    /// `IN` lists, qualified or bare, alias or relation name.
    #[test]
    fn own_path_predicates_bound() {
        assert_eq!(
            classify("SELECT key, value FROM frontmatter WHERE path LIKE 'tasks/%'"),
            regions(&["tasks"])
        );
        assert_eq!(
            classify("SELECT * FROM doc WHERE path = 'tasks/x.md'"),
            regions(&["tasks/x.md"])
        );
        assert_eq!(
            classify("SELECT f.value FROM frontmatter AS f WHERE f.path LIKE 'a/b/c%'"),
            regions(&["a/b"])
        );
        assert_eq!(
            classify("SELECT * FROM task WHERE task.path IN ('a/1.md', 'b/2.md')"),
            regions(&["a/1.md", "b/2.md"])
        );
        assert_eq!(
            classify(
                "SELECT text FROM body WHERE path = 'notes.md' AND section_seq = 3 ORDER BY seq LIMIT 5"
            ),
            regions(&["notes.md"])
        );
    }

    /// The forced counterexample, kept as a test (card set-premises step 4):
    /// `backlink WHERE path = 'tasks/x.md'` is path-constrained and still
    /// WORLD — `backlink.path` names the DESTINATION; a link written in
    /// `agents/y.md` changes the result while `tasks/` never moves. The
    /// WHERE clause never rescues a corpus-fact relation.
    #[test]
    fn backlink_is_world_whatever_the_where_says() {
        assert_eq!(
            classify("SELECT src_path FROM backlink WHERE path = 'tasks/x.md'"),
            SqlProvenance::World
        );
    }

    /// The rest of the world catalog: dangling ("no file exists" readings),
    /// link (corpus-resolved columns), the `.base` relations (outside the
    /// fingerprint domain), and anything unknown.
    #[test]
    fn corpus_fact_relations_are_world() {
        for sql in [
            "SELECT * FROM dangling WHERE src_path LIKE 'tasks/%'",
            "SELECT target_raw FROM link WHERE src_path = 'a/x.md'",
            "SELECT * FROM base WHERE path = 'b.base'",
            "SELECT * FROM _meridian_view",
            "SELECT * FROM duckdb_tables()",
            "SELECT * FROM hist WHERE path = 'a.md'",
        ] {
            assert_eq!(classify(sql), SqlProvenance::World, "{sql}");
        }
    }

    /// Aggregates are world BY LAW (plan §4.5's letter), whatever the WHERE
    /// says — and the whitelist direction catches every unlisted function,
    /// so no aggregate spelling can slip through as bounded.
    #[test]
    fn aggregates_and_unknown_functions_are_world() {
        for sql in [
            "SELECT count(*) FROM doc WHERE path LIKE 'tasks/%'",
            "SELECT max(bytes) FROM doc WHERE path LIKE 'tasks/%'",
            "SELECT path FROM doc WHERE path LIKE 'tasks/%' GROUP BY path",
            "SELECT regr_slope(bytes, line_count) FROM doc WHERE path LIKE 'tasks/%'",
            "SELECT row_number() OVER () FROM doc WHERE path LIKE 'tasks/%'",
        ] {
            assert_eq!(classify(sql), SqlProvenance::World, "{sql}");
        }
    }

    /// Unprovable shapes escalate: joins, subqueries, CTEs, set ops,
    /// top-level OR, predicates not on the own-path column, LIKE forms with
    /// inner wildcards, missing WHERE, quoted identifiers.
    #[test]
    fn unboundable_shapes_escalate_to_world() {
        for sql in [
            "SELECT * FROM doc",
            "SELECT * FROM doc d JOIN section s ON d.path = s.path WHERE d.path LIKE 'a/%'",
            "SELECT * FROM doc, section WHERE doc.path LIKE 'a/%'",
            "SELECT * FROM doc WHERE path IN (SELECT path FROM section)",
            "WITH x AS (SELECT * FROM doc) SELECT * FROM x WHERE path = 'a.md'",
            "SELECT * FROM doc WHERE path LIKE 'a/%' OR bytes > 10",
            "SELECT * FROM doc WHERE bytes > 10",
            "SELECT * FROM doc WHERE path LIKE 'a/%b/%'",
            "SELECT * FROM doc WHERE path LIKE 'tasks/x_.md'",
            "SELECT * FROM doc WHERE path LIKE '%'",
            "SELECT * FROM section WHERE heading = 'tasks/x.md'",
            "SELECT * FROM doc WHERE lower(path) = 'a.md'",
            "SELECT * FROM \"doc\" WHERE path = 'a.md'",
            "SELECT * FROM doc WHERE path = 'a.md' UNION SELECT * FROM doc",
            "SELECT * FROM doc WHERE path = 'a.md' /* unterminated",
            "SELECT * FROM doc WHERE path = 'unterminated",
        ] {
            assert_eq!(classify(sql), SqlProvenance::World, "{sql}");
        }
    }

    /// A recognized bound rides ANY position among the conjuncts, an OR
    /// buried inside parentheses of a NON-granting conjunct stays harmless,
    /// and scalar whitelisted functions elsewhere do not spoil the grant.
    #[test]
    fn conjunct_scan_is_positional_and_paren_aware() {
        assert_eq!(
            classify(
                "SELECT * FROM frontmatter WHERE (key = 'a' OR key = 'b') AND path LIKE 'x/%' AND lower(value) = 'z'"
            ),
            regions(&["x"])
        );
        assert_eq!(
            classify("SELECT * FROM doc WHERE bytes > 10 AND path = 'a/b.md'"),
            regions(&["a/b.md"])
        );
    }

    /// The premise seam: bounded provenance names its regions as scope
    /// paths; world names the root (the empty path) — every scope folds
    /// against the TREE, and no journal appears anywhere in this module's
    /// surface (the no-journal assertion, structural half).
    #[test]
    fn scopes_name_tree_premises() {
        assert_eq!(
            classify("SELECT * FROM doc WHERE path LIKE 'tasks/%'").scopes(),
            vec!["tasks".to_owned()]
        );
        assert_eq!(SqlProvenance::World.scopes(), vec![String::new()]);
    }
}
