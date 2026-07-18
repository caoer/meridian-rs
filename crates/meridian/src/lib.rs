//! # meridian — the markdown world model, as one lib
//!
//! **Charter:** dialect syntax, the governed node tree, and disk truth —
//! three internal modules behind one curated `pub` surface; every op the
//! sidecar answers is a call into this lib.
//!
//! Cargo's doctrine, transposed: *"each command is a thin wrapper around
//! ops"* — here, each wire op is a thin wrapper around a lib call. The
//! modules carry the same charters the rival candidate expresses as crates;
//! this candidate holds them with module discipline + review instead of the
//! crate graph (RFC-1318 school: a boundary is cost until a second consumer,
//! an unbreakable-by-construction invariant, or a distinct stability contract
//! earns it — see the branch README's satellite map for the split triggers).
//!
//! **The three laws** (binding, vision §2):
//! 1. Rust is the world model, Go is the actor — this lib never talks to an
//!    agent; Go never parses markdown.
//! 2. Disk = markdown, the only durable truth; this lib's memory is derived
//!    and disposable. **No persistence anywhere**: no snapshot files, no
//!    cache files, no database, by charter in every module below.
//! 3. One-way arrows: files → Rust → Go. Recovery at every layer is
//!    re-derive from the layer below (cold rebuild: 2.16 s measured).
//!
//! **Module rung map:** [`syntax`] rung 1 · [`model`] rungs 1–3 (rungs 5–6
//! borrow its tree and index) · [`fs`] rungs 1 and 4 (watch).
//!
//! **Never in this lib:** protocol types or framing (the `wire` crate is the
//! only Go-visible surface), body formatting (ccc-mdformat owns it,
//! permanently), executing agent work, persistence of any kind.

pub mod fs;
pub mod model;
pub mod syntax;

// Curated surface: the seams the sidecar (and later the daemon) wire up.
// Everything not re-exported here is module-internal by intent.
pub use fs::read_file;
pub use model::{
    build, resolve, validate_splice, CasMismatch, FrontmatterMap, Node, NodeKind, NodeRev, Ref,
    ResolveError, Span, SpliceGuard, Target, Tree,
};
pub use syntax::{parse, SyntaxEvent, SyntaxEventKind};
