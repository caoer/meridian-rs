//! OD7 refresh-failure observability — daemon-memory `RefreshState`.
//!
//! Telemetry, never correctness: failed rebuild cannot write last-good
//! `view.duckdb`, so errors live here and surface on `view_path` as
//! `refresh_in_progress` + `last_error`. Never gate freshness (§Q3 fold).
//! Lost on restart. Synchronous half only — no async retry/backoff fields.

/// The per-workspace refresh telemetry the daemon holds in memory (OD7). One
/// entry per workspace that has attempted a build; absent until the first
/// `view_path` publish.
#[derive(Debug, Clone, Default)]
pub(crate) struct RefreshState {
    /// Fingerprint of the last successful publish — daemon-memory proxy for
    /// the published file's `_meridian_view` stamp `as_of`. The daemon is the
    /// sole persistent writer (OD6), so within one epoch this equals the
    /// on-disk stamp; `view_path` uses it as the pre-open `as_of` hint.
    pub last_ok_fingerprint: Option<model::MerkleRoot>,
    /// Unix seconds of the last successful publish.
    pub last_ok_unix: Option<u64>,
    /// The last rebuild failure, projected to the wire shape once so the reply
    /// clones it directly.
    pub last_error: Option<wire::RefreshError>,
    /// Consecutive failures since the last success — reset to `0` on success.
    pub consecutive_failures: u32,
}

impl RefreshState {
    /// Record a successful publish.
    pub(crate) fn record_ok(&mut self, fingerprint: model::MerkleRoot, unix: u64) {
        self.last_ok_fingerprint = Some(fingerprint);
        self.last_ok_unix = Some(unix);
        self.last_error = None;
        self.consecutive_failures = 0;
    }

    /// Record a failed publish. The last-good fingerprint stays untouched — the
    /// old file is still published.
    pub(crate) fn record_err(&mut self, error: wire::RefreshError) {
        self.last_error = Some(error);
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
    }
}

/// Project a [`view::ViewError`] onto the OD7 wire [`wire::RefreshError`] class.
/// `fingerprint_attempted` is the fingerprint the build targeted.
pub(crate) fn view_err_to_refresh(
    error: &view::ViewError,
    attempted: &model::MerkleRoot,
    unix: u64,
) -> wire::RefreshError {
    // All `Io`-class at round-1's grain. A corpus `ParseError` cannot arise on
    // the publish path: the parse already succeeded in `warm_or_build`.
    let code = match error {
        view::ViewError::Duckdb(_)
        | view::ViewError::Io(_)
        | view::ViewError::WalPresent(_)
        | view::ViewError::EphemeralDrawer => wire::RefreshErrorCode::Io,
    };
    wire::RefreshError {
        code,
        unix,
        fingerprint_attempted: Some(wire::Root(attempted.0.clone())),
        message: error.to_string(),
    }
}
