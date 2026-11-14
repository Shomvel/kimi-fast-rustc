//! Read-only graph facts. A graph edit or compaction invalidates these snapshots.
pub mod cfg;
pub mod cost;
pub(crate) mod demanded_bits;
pub mod dominators;
pub mod function_attrs;
pub(crate) mod induction;
pub(crate) mod known_bits;
pub mod loops;
pub mod memory;
pub mod ranges;
pub mod refinement;
pub mod uses;
pub(crate) mod value_evaluation;
