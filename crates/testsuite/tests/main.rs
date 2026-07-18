//! The consolidated integration-test binary (matklad doctrine): one binary,
//! modules per concern. Rung 1 adds `gt_parse` (syntax/model vs the GT pack)
//! and `wire_golden` (contract example exchanges verbatim) as modules here.

mod gt_pack_smoke;
mod wire_vocab;
