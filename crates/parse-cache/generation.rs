//! The locked production dependency closure of the semantic source crates.
//! Workspace package version numbers and unrelated packages do not enter it.
use std::collections::BTreeSet;

pub(crate) const ROOTS: [&str; 4] = ["addr", "syntax", "model", "parse-cache"];

pub(crate) fn dependency_bytes(lock: &str) -> Vec<u8> {
    let lock: toml::Value = lock.parse().expect("valid Cargo.lock");
    let packages = lock["package"].as_array().expect("lock packages");
    let mut todo: Vec<usize> = packages
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let name = p["name"].as_str().expect("package name");
            (ROOTS.contains(&name) && p.get("source").is_none()).then_some(i)
        })
        .collect();
    assert_eq!(
        todo.len(),
        ROOTS.len(),
        "all semantic workspace roots are locked"
    );
    let mut seen = BTreeSet::new();
    while let Some(i) = todo.pop() {
        if !seen.insert(i) {
            continue;
        }
        for dependency in packages[i]
            .get("dependencies")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
        {
            let value = dependency.as_str().expect("locked dependency");
            let mut parts = value.split_whitespace();
            let name = parts.next().expect("dependency name");
            let version = parts.next();
            let source = parts
                .next()
                .map(|s| s.trim_start_matches('(').trim_end_matches(')'));
            let matches: Vec<_> = packages
                .iter()
                .enumerate()
                .filter_map(|(j, p)| {
                    (p["name"].as_str() == Some(name)
                        && version.is_none_or(|v| p["version"].as_str() == Some(v))
                        && source.is_none_or(|s| {
                            p.get("source").and_then(toml::Value::as_str) == Some(s)
                        }))
                    .then_some(j)
                })
                .collect();
            assert_eq!(matches.len(), 1, "unambiguous dependency: {value}");
            todo.push(matches[0]);
        }
    }
    let mut selected: Vec<_> = seen
        .into_iter()
        .filter(|i| {
            let external = packages[*i].get("source").is_some();
            assert!(
                external || ROOTS.contains(&packages[*i]["name"].as_str().unwrap()),
                "a local semantic dependency requires source coverage in generation::ROOTS"
            );
            external
        })
        .map(|i| {
            // Dependency spellings can gain version qualifiers when an
            // unrelated package introduces another version. Hash resolved
            // identities, not those context-dependent spellings.
            let fields = ["name", "version", "source", "checksum"]
                .into_iter()
                .filter_map(|key| packages[i].get(key).map(|v| (key.to_owned(), v.clone())))
                .collect();
            toml::Value::Table(fields).to_string()
        })
        .collect();
    selected.sort();
    selected.join("\n").into_bytes()
}
