//! Spec-check core: the capture/inventory/check engine.
//!
//! This crate is the ported engine from the research repository
//! (`research/tools/cargo-spec-check`, lineage revision `5d54c0690208`).
//! Module names and behaviour are preserved so that archived campaign
//! evidence keeps replaying against this engine bit-for-bit.
//!
//! Internal re-organisation is incremental; see `docs/ARCHITECTURE.md`
//! for the target layout and the extension recipe for new analyses.

pub mod assertion_rule;
pub mod attribution;
pub mod borrowed_resource;
pub mod callback;
pub mod call_guidance;
pub mod capture;
pub mod census;
pub mod client;
pub mod conditional;
pub mod conditional_check;
pub mod conditional_project;
pub mod contract_scan;
pub mod contracts;
pub mod demand;
pub mod demand_check;
pub mod evidence;
pub mod filter_guidance;
pub mod finite;
pub mod ghost_abi;
pub mod guard_owner;
pub mod guidance;
pub mod handoff;
pub mod helper;
pub mod ir;
pub mod ktest;
pub mod link;
pub mod literal;
pub mod obligation;
pub mod option_alternative;
pub mod ordinary;
pub mod owner_post;
pub mod owner_report_enrich;
pub mod permission_guidance;
pub mod prefix;
pub mod project;
pub mod reader;
pub mod requires;
pub mod resource;
pub mod resource_flow;
pub mod resource_project;
pub mod return_forwarding;
pub mod review;
pub mod runner;
pub mod scope;
pub mod set_state;
pub mod state;
pub mod state_semantics;
pub mod std_body;
pub mod support;
pub mod test_census;
pub mod vir;

pub use serde_json::json;
pub use support::{execute, hash, write_json, Result};
