//! The ONE held-token family classifier every ingress door shares — the
//! priced cutover protocol's §3.1 demand ("classification cannot be copied
//! into seven comparison sites") made a module. wire-contract §5.7/§12.3:
//! three errors, three facts, never flattened.
//!
//! A door calls [`family_refusal`] BEFORE its own digest compare: a KNOWN
//! RETIRED family refuses `fingerprint_version_retired` even when its digest
//! would also compare unequal (the premise did not move — the LAW moved); an
//! UNKNOWN FUTURE family refuses `fingerprint_version_unsupported` with its
//! own teaching, never a fallthrough to retired. `None` means the token is
//! current-family, older-domain (prefixes differ by construction — the
//! ordinary mismatch delivers itself), or not a root token at all: the
//! door's own compare stays authoritative for all three.

use wire::ErrorBody;

/// Classify `token` against `serving`; mint the version refusal when its
/// FAMILY is not current. `scope` is the premise's scope spelling for the
/// re-mint teaching (`"/"` for the workspace-root premise).
#[must_use]
pub fn family_refusal(
    root: &fs::WorkspaceRoot,
    token: &str,
    scope: &str,
    serving: model::ServingVersion,
) -> Option<Box<ErrorBody>> {
    match serving.classify(token) {
        model::RootTokenVerdict::Retired => Some(with_cutover_facts(
            root,
            ErrorBody::version_retired(scope),
            token,
            serving,
        )),
        model::RootTokenVerdict::UnsupportedFuture => Some(with_cutover_facts(
            root,
            ErrorBody::version_unsupported(scope),
            token,
            serving,
        )),
        model::RootTokenVerdict::Current
        | model::RootTokenVerdict::DomainMoved
        | model::RootTokenVerdict::Malformed => None,
    }
}

/// Gate-4 mandatory facts on a version refusal (merkle-spec §4.2.6): the
/// held token's decoded two-dimensional version (absent when undecodable —
/// an unknown future family has no identity here), the serving version, and
/// the B-04 record's stable cutover id.
fn with_cutover_facts(
    root: &fs::WorkspaceRoot,
    base: ErrorBody,
    token: &str,
    serving: model::ServingVersion,
) -> Box<ErrorBody> {
    let received = model::parse_root(token)
        .and_then(|parsed| serving.decode(parsed.position))
        .map(|v| wire::FingerprintVersion {
            hash_law: v.hash_law,
            domain: v.domain,
        });
    let current = wire::FingerprintVersion {
        hash_law: serving.current.hash_law,
        domain: serving.current.domain,
    };
    let cutover_id = match fs::cutover::status(root) {
        Ok(fs::cutover::CutoverStatus::Present(rec)) => Some(rec.cutover_id),
        _ => None,
    };
    let mut e = base;
    e.received_fingerprint_version = received;
    e.current_fingerprint_version = Some(current);
    e.cutover_id = cutover_id;
    Box::new(e)
}
