//! Allocator forwarding symbols, generated from rustc's allocator contract.
//!
//! Custom global allocator implementations are already emitted as mono items.
//! Only rustc's requested default allocator/error-handler methods need wrappers.

use kfrc_x86::{FunctionCode, MachineFunction, MachineInstr, SymbolBinding};
use rustc_ast::expand::allocator::{NO_ALLOC_SHIM_IS_UNSTABLE, default_fn_name, global_fn_name};
use rustc_codegen_ssa::base::{allocator_kind_for_codegen, allocator_shim_contents};
use rustc_codegen_ssa::{CompiledModule, ModuleKind};
use rustc_middle::mir::mono::CodegenUnitNameBuilder;
use rustc_middle::ty::TyCtxt;
use rustc_session::config::OutputType;
use rustc_span::def_id::LOCAL_CRATE;
use rustc_symbol_mangling::mangle_internal_symbol;

pub(crate) fn codegen(tcx: TyCtxt<'_>) -> Option<CompiledModule> {
    let kind = allocator_kind_for_codegen(tcx)?;
    let mut functions = Vec::new();
    for method in allocator_shim_contents(tcx, kind) {
        let name = mangle_internal_symbol(tcx, &global_fn_name(method.name));
        let target = mangle_internal_symbol(tcx, &default_fn_name(method.name));
        let mut function = MachineFunction::new(name);
        function.symbols.push(target);
        // Every allocator input is a usize/pointer (Layout is two usizes),
        // and every allocator return is a pointer, unit, or never. Passing
        // the argument registers unchanged exactly preserves that ABI.
        function.instructions = vec![MachineInstr::Call { target: 0 }, MachineInstr::Ret];
        functions.push(FunctionCode {
            function,
            binding: SymbolBinding::Global { hidden: true },
        });
    }

    let mut marker = MachineFunction::new(mangle_internal_symbol(tcx, NO_ALLOC_SHIM_IS_UNSTABLE));
    marker.instructions.push(MachineInstr::Ret);
    functions.push(FunctionCode {
        function: marker,
        binding: SymbolBinding::Global { hidden: true },
    });

    let name = CodegenUnitNameBuilder::new(tcx)
        .build_cgu_name(LOCAL_CRATE, &["crate"], Some("allocator"))
        .to_string();
    let outputs = tcx.output_filenames(());
    let path = outputs.temp_path_for_cgu(
        OutputType::Object,
        &name,
        tcx.sess.invocation_temp.as_deref(),
    );
    let bytes = kfrc_x86::write_object(&functions)
        .unwrap_or_else(|error| tcx.dcx().fatal(format!("kfrc: allocator shim: {error}")));
    std::fs::write(&path, bytes).unwrap_or_else(|error| {
        tcx.dcx()
            .fatal(format!("kfrc: writing {}: {error}", path.display()))
    });
    Some(CompiledModule {
        name,
        kind: ModuleKind::Allocator,
        object: Some(path),
        dwarf_object: None,
        bytecode: None,
        assembly: None,
        llvm_ir: None,
        links_from_incr_cache: Vec::new(),
    })
}
