//! Read-only graph facts. A graph edit or compaction invalidates these snapshots.
pub mod cfg;
pub mod dominators;
pub mod memory;
pub mod refinement;
pub mod uses;
