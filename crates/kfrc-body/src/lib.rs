//! The Typed Body contract (`docs/design/frontend/body.md`) and the
//! `TypeOracle` seam (`docs/plan/poc/architecture.md`).
//!
//! Producers: the PoC's MIR → Body converter (in `rustc_codegen_kfrc`) and,
//! later, kfrc's own front end. Consumers: the SoN builder in `kfrc-son`.
//! Nothing here depends on rustc; types are `TyId`s answered by a
//! `TypeOracle`.

pub mod body;
pub mod oracle;
pub mod types;
pub mod verify;

pub use body::*;
pub use oracle::*;
pub use types::*;
