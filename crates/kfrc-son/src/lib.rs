//! The Rust-semantic sea-of-nodes IR (`docs/design/ir/`).
//!
//! `build_graph` retains Rust values until the explicit `lower_layout` stage.
//! This is the sole production IR pipeline. Eligible definitions use immutable
//! polymorphic R and demand-driven specialization; concrete Body input remains
//! supported for constructs not yet handled by the definition producer.
//! Unsupported constructs are rejected with `BuildError::Unsupported`.

pub mod analysis;
pub mod arithmetic;
pub mod intrinsic;
pub use intrinsic::IntegerIntrinsicPayload;
pub mod build;
pub mod graph;
pub mod interpret;
pub mod lower_layout;
pub mod optimize;
pub mod pipeline;
pub mod semantic_value;
pub mod specialize;
mod transforms;
pub mod verify;

pub use arithmetic::ArithmeticFlags;
pub use build::{BuildError, build_graph};
pub use graph::*;
pub use interpret::{
    InterpretError, Interpreter, InterpreterThread, Outcome, SemanticCallHandler, SemanticMemory,
    SemanticOutcome, Value,
};
pub use lower_layout::{
    LayoutError, LayoutOptions, LayoutStats, lower_layout, lower_layout_with_options,
};
pub use optimize::{OptimizeStats, optimize_graph};
pub use pipeline::{PipelineError, optimize_specialized_graph, optimize_specialized_seams};
pub use transforms::ipo::{
    GraphProvider, IdentityFoldStats, InlineBudget, InlineCostPolicy, InlineDecision, InlineError,
    InlineSkip, InlineStats, fold_identity_calls, inline_calls, inline_calls_with_cost,
};
pub use transforms::scalar::ranges::{RangeFoldStats, run as fold_scalar_ranges};
pub use verify::{VerifyError, verify_graph};
