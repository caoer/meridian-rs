//! Read-only source-corpus measurement; the second argument is a disposable
//! output file outside the source tree. Optional third argument samples evenly.
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn walk(root: &Path, paths: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if entry.file_name().as_encoded_bytes().starts_with(b".") || kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            walk(&entry.path(), paths)?;
        } else if entry.path().extension().is_some_and(|e| e == "md") {
            paths.push(entry.path());
        }
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() < 3 {
        return Err(io::Error::other(
            "profile SOURCE_DIR OUTPUT_FILE [SAMPLE_COUNT]",
        ));
    }
    let source = Path::new(&args[1]).canonicalize()?;
    let output = Path::new(&args[2]);
    if output.starts_with(&source) {
        return Err(io::Error::other("output must be outside source"));
    }
    let mut paths = Vec::new();
    walk(&source, &mut paths)?;
    paths.sort();
    let total = paths.len();
    if let Some(sample) = args
        .get(3)
        .and_then(|s| s.to_str())
        .and_then(|s| s.parse::<usize>().ok())
        && sample > 0
        && sample < total
    {
        paths = (0..sample)
            .map(|i| paths[i * total / sample].clone())
            .collect();
    }
    let start = Instant::now();
    let chunks = paths.chunks(paths.len().div_ceil(8).max(1));
    let docs = std::thread::scope(|scope| {
        let workers: Vec<_> = chunks
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .filter_map(|p| fs::read_to_string(p).ok())
                        .map(|raw| {
                            let nodes = syntax::parse(&raw);
                            model::build(raw, nodes)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("parse worker"))
            .collect::<Vec<_>>()
    });
    let parse = start.elapsed();
    let raw_bytes: usize = docs.iter().map(|d| d.raw.len()).sum();
    let start = Instant::now();
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(output)?;
    let mut writer = BufWriter::new(file);
    let mut encoded = 0u64;
    let mut count = 0u64;
    for doc in &docs {
        if let Some(bytes) = parse_cache::encode(doc) {
            writer.write_all(&(bytes.len() as u64).to_le_bytes())?;
            writer.write_all(&model::leaf_digest(doc.raw.as_bytes()))?;
            writer.write_all(&bytes)?;
            encoded += bytes.len() as u64;
            count += 1;
        }
    }
    writer.flush()?;
    let encode = start.elapsed();
    drop(writer);
    drop(docs);
    let start = Instant::now();
    let mut reader = BufReader::new(File::open(output)?);
    let mut restored = Vec::new();
    for _ in 0..count {
        let mut len = [0; 8];
        let mut digest = [0; 32];
        reader.read_exact(&mut len)?;
        reader.read_exact(&mut digest)?;
        let mut bytes = vec![0; usize::try_from(u64::from_le_bytes(len)).unwrap()];
        reader.read_exact(&mut bytes)?;
        restored.push(
            parse_cache::decode(&bytes, &digest).ok_or_else(|| io::Error::other("codec miss"))?,
        );
    }
    println!(
        "source_members={total} sampled={} cached={count} raw_bytes={raw_bytes} encoded_bytes={encoded} parse_parallel_s={:.3} encode_write_s={:.3} decode_sequential_s={:.3}",
        paths.len(),
        parse.as_secs_f64(),
        encode.as_secs_f64(),
        start.elapsed().as_secs_f64()
    );
    Ok(())
}
