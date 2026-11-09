//! Machine instructions, allocation, encoding and reference execution.
//!
//! This layer consumes lowered virtual instructions. It has no dependency on
//! a source-language graph or the historical root compiler pipeline.

pub mod allocate;
pub mod eval;
pub mod regalloc;
pub mod x86;
pub mod x86_encode;
