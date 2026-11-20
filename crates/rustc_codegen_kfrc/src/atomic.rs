//! Crate-private atomic runtime routines. The semantic call remains an unknown
//! read/write barrier; only referenced routines are emitted, with ordinary CGU
//! ownership and promotion. The crate disambiguator prevents cross-crate aliasing.

use kfrc_cranelift::AtomicOperation;
use rustc_middle::ty::TyCtxt;
use rustc_symbol_mangling::mangle_internal_symbol;
use std::collections::{BTreeMap, BTreeSet};

use crate::{codegen, emission, machine};

pub(crate) fn symbol(
    tcx: TyCtxt<'_>,
    operation: AtomicOperation,
    bits: u8,
    signed: bool,
) -> String {
    mangle_internal_symbol(
        tcx,
        &format!(
            "__kfrc_atomic_{}_{}{bits}",
            operation.name(),
            if signed { "i" } else { "u" }
        ),
    )
}

pub(crate) fn register(
    tcx: TyCtxt<'_>,
    graphs: &codegen::CrateGraphs,
    jobs: &mut [emission::CguJob],
    adapters: &mut BTreeMap<String, machine::CompiledFunction>,
) -> Result<(), codegen::CompileError> {
    let references: BTreeSet<_> = graphs
        .bodies
        .values()
        .flat_map(|graph| graph.callees.iter().map(|callee| callee.symbol.as_str()))
        .collect();
    for operation in [
        AtomicOperation::Load,
        AtomicOperation::Store,
        AtomicOperation::Sub,
        AtomicOperation::Add,
        AtomicOperation::Exchange,
        AtomicOperation::CompareExchange,
        AtomicOperation::Fence,
    ] {
        for bits in [8, 16, 32, 64] {
            for signed in [false, true] {
                if operation == AtomicOperation::Fence && (bits != 64 || signed) {
                    continue;
                }
                let symbol = symbol(tcx, operation, bits, signed);
                if !references.contains(symbol.as_str()) {
                    continue;
                }
                let function = machine::atomic_runtime(&symbol, operation, bits, signed).map_err(
                    |message| codegen::CompileError {
                        symbol: symbol.clone(),
                        message,
                    },
                )?;
                jobs[0]
                    .pending_functions
                    .insert(symbol.clone(), kfrc_object::SymbolBinding::Local);
                adapters.insert(symbol, (function, Vec::new()));
            }
        }
    }
    Ok(())
}
