//! Cranelift machine layer for verified or internally trusted Layout graphs.
//!
//! The graph is scheduled by `kfrc_son::schedule` (the same schedule every
//! machine layer consumes), exported to Cranelift IR, and compiled by
//! Cranelift: mid-end rewriting, instruction selection, register allocation
//! and encoding. The result is an `EncodedFunction`, the machine-independent
//! record the object writer accepts.
//!
//! Layout values keep their own integer width in Cranelift IR: a `bool` or
//! `u8` is an `i8`, a pointer an `i64`. Signedness lives in the operations
//! (extension kind, comparison predicate, division), exactly as in the graph.
//! Narrow arguments and return values are extended at ABI boundaries by the
//! signature, so callers and callees compiled by another backend may rely on
//! either convention.
//!
//! Symbol addresses are RIP-relative and calls are rel32, bound by the linker
//! directly or through the PLT. Thread-local addresses use the canonical
//! general-dynamic sequence, which the linker relaxes to local-exec in an
//! executable.

mod emit;
mod export;
mod machine;

use cranelift_codegen::Context;
use cranelift_codegen::ir;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use kfrc_body::EncodedFunction;
use kfrc_son::lower_layout::LayoutOptions;
use kfrc_son::schedule::Schedule;
use kfrc_son::{FunctionGraph, GraphLevel};

pub use export::LowerError;
pub use machine::{Machine, MachineError, MachineOptions, Optimization};

pub fn lower_function(
    graph: &FunctionGraph,
    machine: &Machine,
) -> Result<EncodedFunction, LowerError> {
    lower_function_with_options(graph, machine, LayoutOptions::default())
}

pub fn lower_function_with_options(
    graph: &FunctionGraph,
    machine: &Machine,
    options: LayoutOptions,
) -> Result<EncodedFunction, LowerError> {
    let mut context = Context::new();
    let symbols = export_function(graph, machine, options, &mut context.func)?;
    let dump = std::env::var("KFRC_DUMP_MACHINE")
        .ok()
        .filter(|pattern| graph.symbol.contains(pattern.as_str()));
    if dump.is_some() {
        eprintln!("cranelift ir {} {{\n{}}}", graph.symbol, context.func.display());
        context.set_disasm(true);
    }
    if machine.verify {
        cranelift_codegen::verify_function(&context.func, &*machine.isa).map_err(|errors| {
            LowerError::Malformed(format!(
                "Cranelift IR verification failed: {errors}\n{}",
                context.func.display()
            ))
        })?;
    }
    let function = emit::encode(&graph.symbol, &mut context, &symbols, machine)?;
    if dump.is_some()
        && let Some(text) = context
            .compiled_code()
            .and_then(|compiled| compiled.vcode.as_ref())
    {
        eprintln!("cranelift machine {} {{\n{text}}}", graph.symbol);
    }
    Ok(function)
}

/// Builds the Cranelift IR of `graph` into `function` and returns the symbol
/// table its external names index (`UserExternalName { namespace: 0, index }`).
pub fn export_function(
    graph: &FunctionGraph,
    machine: &Machine,
    options: LayoutOptions,
    function: &mut ir::Function,
) -> Result<Vec<String>, LowerError> {
    if !graph.state.is_ready_for_lowering() {
        return Err(LowerError::NotVerified);
    }
    if graph.level != GraphLevel::Layout {
        return Err(LowerError::Unsupported(
            "semantic graphs must pass through lower_layout before machine lowering".into(),
        ));
    }
    let schedule = Schedule::new(graph).map_err(|error| LowerError::Malformed(error.0))?;
    let mut builder_context = FunctionBuilderContext::new();
    let builder = FunctionBuilder::new(function, &mut builder_context);
    export::Exporter::new(
        graph,
        &schedule,
        builder,
        options,
        machine.isa.frontend_config(),
        machine.options,
    )
    .run()
}
