//! The Rust-semantic sea-of-nodes IR (`docs/design/ir/`).
//!
//! P0 scope (`docs/plan/poc/phases.md`): R-level graphs for scalar code —
//! integers and `bool`, control flow, direct calls — built from a Typed Body,
//! checked by the verifier and executable by the reference interpreter.
//! Constructs beyond that are rejected with `BuildError::Unsupported`.

pub mod build;
pub mod graph;
pub mod interpret;
pub mod verify;

pub use build::{BuildError, build_graph};
pub use graph::*;
pub use interpret::{InterpretError, Interpreter, Outcome};
pub use verify::{VerifyError, verify_graph};
