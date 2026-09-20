//! A parser/model semantic key, independent of unrelated daemon edits and
//! workspace package version bumps. Follow the semantic crates' locked
//! dependency closure; exclude unrelated and workspace-version-only entries.
use std::fs;
use std::path::{Path, PathBuf};

mod generation;

fn sources(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read semantic source directory") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            sources(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
}

fn main() {
    let manifest =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let root = manifest
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let mut files = Vec::new();
    for name in generation::ROOTS {
        let dir = root.join("crates").join(name);
        println!("cargo:rerun-if-changed={}", dir.join("src").display());
        sources(&dir.join("src"), &mut files);
        files.push(dir.join("Cargo.toml"));
    }
    files.push(manifest.join("build.rs"));
    files.push(manifest.join("generation.rs"));
    files.sort();
    let mut hash = blake3::Hasher::new();
    hash.update(b"mrd-document-semantics-v1\0");
    for path in files {
        println!("cargo:rerun-if-changed={}", path.display());
        hash.update(
            path.strip_prefix(root)
                .expect("local source")
                .as_os_str()
                .as_encoded_bytes(),
        );
        hash.update(&[0]);
        let bytes = fs::read(&path).expect("read semantic source");
        hash.update(&(bytes.len() as u64).to_le_bytes());
        hash.update(&bytes);
    }
    let lock = root.join("Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock.display());
    let lock = fs::read_to_string(lock).expect("locked dependencies");
    hash.update(&generation::dependency_bytes(&lock));
    println!(
        "cargo:rustc-env=MRD_PARSE_GENERATION={}",
        hash.finalize().to_hex()
    );
}
