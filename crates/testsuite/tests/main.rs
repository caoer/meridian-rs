//! The consolidated integration-test binary (matklad doctrine): one binary,
//! modules per concern. Rung 1 adds `gt_parse` (syntax/model vs the GT pack)
//! and `wire_golden` (contract example exchanges verbatim) as modules here.

mod charset_guard;
mod delta_e3e4;
mod gt_pack_smoke;
mod gt_parse;
mod pf_frozen_sweep;
mod u0_read_parity;
mod u4a1_render_parity;
mod u4a2_composed_read;
mod u4b_real_lock;
mod walk_resolve;
mod wire_vocab;
mod wsfix_oracle;
