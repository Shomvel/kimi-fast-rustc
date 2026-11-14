//! Compatibility exports for the original optimization entry point.
//!
//! Pipeline orchestration lives in [`crate::pipeline`]; integer rules live in
//! the scalar transform module.

pub use crate::pipeline::{OptimizeStats, optimize_graph};
