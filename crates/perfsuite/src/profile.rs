//! Profiles are data: construct incidence/intensity, size distribution,
//! pathology rates. A `(profile, seed, files)` triple is a [`Recipe`] — the
//! committed form of a corpus of any size. Profiles come from `profiles/*.toml`
//! (hand-seeded from the lane0 survey) or from `corpusgen fit <vault>`
//! (statistics only, no content — committable even from a private vault).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Bumped whenever generation logic changes shape: invalidates corpus caches
/// and travels in every manifest as provenance.
///
/// v2: task-line indentation clamped to a real open-item context. Without this
/// bump a cached v1 corpus keeps serving pre-fix bytes — `ensure` reuses on
/// (recipe hash, this constant), and neither sees a generator code change.
pub const GENERATOR_VERSION: u32 = 2;

/// Construct ids the generator can plant — 1:1 with `syntax::DialectKind`
/// discriminants, so inventories join directly against parser output.
pub const KNOWN_CONSTRUCTS: [&str; 11] = [
    "frontmatter",
    "heading",
    "fence",
    "inline_code",
    "anchor",
    "wikilink",
    "embed",
    "callout",
    "task",
    "table",
    "comment",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    /// Where the numbers came from — surveys and fits, never vibes.
    pub provenance: String,
    /// Default corpus file count; recipes may override.
    pub files: u32,
    pub size: SizeDist,
    /// Keys validated against [`KNOWN_CONSTRUCTS`] at load.
    pub constructs: BTreeMap<String, ConstructRate>,
    #[serde(default)]
    pub pathology: Pathology,
    #[serde(default)]
    pub unicode: Unicode,
}

/// Log-normal file-size model: `bytes = median · exp(σ·z)`, clamped to
/// `[64, max_bytes]`. `sigma = 0` makes every file exactly `median_bytes`
/// (adversarial fixed-size profiles).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SizeDist {
    pub median_bytes: u64,
    pub sigma: f64,
    pub max_bytes: u64,
}

/// `file_rate`: fraction of files containing the construct at all.
/// `per_file`: mean count (Poisson) among files that contain it (min 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConstructRate {
    pub file_rate: f64,
    pub per_file: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Pathology {
    /// Per-file probability that the LAST fence is left unterminated (it must
    /// be last: an open fence swallows the rest of the file, and planting
    /// after it would break the ≥-planted inventory invariant).
    pub unterminated_fence_rate: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Unicode {
    /// Probability any given filler word is CJK.
    pub cjk_rate: f64,
}

impl Profile {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let profile: Self =
            toml::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
        profile.validate()?;
        Ok(profile)
    }

    /// Resolve `name_or_path`: a path if it exists or ends in `.toml`,
    /// otherwise `profiles/<name>.toml` next to this crate.
    pub fn resolve(name_or_path: &str) -> Result<Self, String> {
        let direct = Path::new(name_or_path);
        if direct.exists()
            || Path::new(name_or_path)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
        {
            return Self::load(direct);
        }
        let bundled = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("profiles")
            .join(format!("{name_or_path}.toml"));
        Self::load(&bundled)
    }

    pub fn validate(&self) -> Result<(), String> {
        for key in self.constructs.keys() {
            if !KNOWN_CONSTRUCTS.contains(&key.as_str()) {
                return Err(format!(
                    "profile {}: unknown construct '{key}' (known: {KNOWN_CONSTRUCTS:?})",
                    self.name
                ));
            }
        }
        for (key, rate) in &self.constructs {
            // `contains` also rejects NaN/±inf (comparisons are false).
            if !(0.0..=1.0).contains(&rate.file_rate) {
                return Err(format!(
                    "profile {}: {key}.file_rate {} outside [0,1]",
                    self.name, rate.file_rate
                ));
            }
            // A non-finite `per_file` reaches `poisson`, whose Knuth loop never
            // terminates on a NaN cutoff — reject it before generation.
            if !rate.per_file.is_finite() || rate.per_file < 0.0 {
                return Err(format!(
                    "profile {}: {key}.per_file {} must be finite and ≥ 0",
                    self.name, rate.per_file
                ));
            }
        }
        // `max_bytes >= 64`: the size sampler clamps into `[64, max_bytes]`, and
        // `u64::clamp` panics when the lower bound exceeds the upper.
        if self.size.median_bytes == 0
            || self.size.max_bytes < self.size.median_bytes
            || self.size.max_bytes < 64
        {
            return Err(format!(
                "profile {}: degenerate size distribution (need 1 ≤ median ≤ max and max ≥ 64)",
                self.name
            ));
        }
        if !self.size.sigma.is_finite() || self.size.sigma < 0.0 {
            return Err(format!(
                "profile {}: size.sigma {} must be finite and ≥ 0",
                self.name, self.size.sigma
            ));
        }
        if !(0.0..=1.0).contains(&self.pathology.unterminated_fence_rate) {
            return Err(format!(
                "profile {}: pathology.unterminated_fence_rate {} outside [0,1]",
                self.name, self.pathology.unterminated_fence_rate
            ));
        }
        if !(0.0..=1.0).contains(&self.unicode.cjk_rate) {
            return Err(format!(
                "profile {}: unicode.cjk_rate {} outside [0,1]",
                self.name, self.unicode.cjk_rate
            ));
        }
        Ok(())
    }

    pub fn rate(&self, construct: &str) -> Option<&ConstructRate> {
        self.constructs.get(construct)
    }

    /// Stable serialization for recipe hashing (`BTreeMap` keeps key order).
    pub fn canonical(&self) -> String {
        toml::to_string(self).expect("profile serializes")
    }
}

/// FNV-1a 64 — stable across releases/platforms (std's `DefaultHasher` is
/// deliberately not), tiny, dependency-free. Not cryptographic; corpus hashes
/// are integrity receipts, not security boundaries.
#[must_use]
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The committed form of a corpus: profile + seed + file count. Byte-identical
/// regeneration per platform (libm `ln`/`cos` may differ in the last ulp
/// across OS/arch, so cross-platform bit-identity is not promised — see
/// `corpus` docs).
#[derive(Debug, Clone)]
pub struct Recipe {
    pub profile: Profile,
    pub seed: u64,
    pub files: u32,
}

impl Recipe {
    #[must_use]
    pub fn new(profile: Profile, seed: u64, files_override: Option<u32>) -> Self {
        let files = files_override.unwrap_or(profile.files);
        Self {
            profile,
            seed,
            files,
        }
    }

    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut basis = self.profile.canonical();
        let _ = write!(
            basis,
            "|seed={}|files={}|gen=v{GENERATOR_VERSION}",
            self.seed, self.files
        );
        fnv1a64(basis.as_bytes())
    }

    /// Cache-directory name: human-scannable prefix + content hash.
    #[must_use]
    pub fn id(&self) -> String {
        format!(
            "{}-s{}-f{}-{:016x}",
            self.profile.name,
            self.seed,
            self.files,
            self.hash()
        )
    }
}
