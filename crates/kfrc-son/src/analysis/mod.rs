//! Read-only graph facts. A graph edit or compaction invalidates these snapshots.
pub mod cfg;
pub mod cost;
pub mod dominators;
pub mod loops;
pub mod memory;
pub mod ranges;
pub mod refinement;
pub mod uses;
pub(crate) mod known_bits;
