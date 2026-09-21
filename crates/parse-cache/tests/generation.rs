#[path = "../generation.rs"]
mod generation;

const LOCK: &str = r#"
[[package]]
name = "parse-cache"
version = "1.2.0"
dependencies = ["model", "codec-only"]
[[package]]
name = "codec-only"
version = "0.7.0"
source = "git+https://example.test/codec#first"
[[package]]
name = "addr"
version = "1.2.0"
[[package]]
name = "syntax"
version = "1.2.0"
dependencies = ["parser"]
[[package]]
name = "model"
version = "1.2.0"
dependencies = ["syntax", "addr", "hash"]
[[package]]
name = "parser"
version = "0.1.0"
source = "git+https://example.test/parser#one"
dependencies = ["hash"]
[[package]]
name = "hash"
version = "1.0.0"
source = "registry+https://example.test/registry"
checksum = "abc"
[[package]]
name = "unrelated"
version = "9.0.0"
source = "registry+https://example.test/registry"
"#;

#[test]
fn workspace_release_and_unrelated_dependencies_preserve_generation() {
    let baseline = generation::dependency_bytes(LOCK);
    assert_eq!(
        baseline,
        generation::dependency_bytes(&LOCK.replace("1.2.0", "2.0.0"))
    );
    assert_eq!(
        baseline,
        generation::dependency_bytes(&LOCK.replace("9.0.0", "10.0.0"))
    );
}

#[test]
fn a_semantic_dependency_or_its_transitive_dependency_changes_generation() {
    let baseline = generation::dependency_bytes(LOCK);
    assert_ne!(
        baseline,
        generation::dependency_bytes(&LOCK.replace("codec#first", "codec#second"))
    );
    assert_ne!(
        baseline,
        generation::dependency_bytes(&LOCK.replace("#one", "#two"))
    );
    assert_ne!(
        baseline,
        generation::dependency_bytes(&LOCK.replace("checksum = \"abc\"", "checksum = \"def\""))
    );
}
