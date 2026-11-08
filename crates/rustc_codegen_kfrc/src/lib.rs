//! `rustc_codegen_kfrc`: the PoC shell (`docs/plan/poc/architecture.md`).
//!
//! rustc 1.94.1 runs its whole front end and mono-item collection; this
//! backend converts each mono item's MIR into a kfrc Typed Body, builds and
//! verifies an R-level SoN graph, lowers it with `kfrc-x86`, and hands one
//! object per codegen unit back to rustc's linker driver. Only this crate
//! links `rustc_*`; `kfrc-body`, `kfrc-son` and `kfrc-x86` do not.
//!
//! Anything outside the implemented subset is a fatal error naming the
//! function and construct — never a silent fallback to LLVM. Crates that
//! need unsupported features are built with the LLVM backend instead and
//! marked as such (`docs/plan/poc/scope.md`).

#![feature(rustc_private)]

extern crate rustc_abi;
extern crate rustc_codegen_ssa;
extern crate rustc_data_structures;
extern crate rustc_hir;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;
extern crate rustc_target;

// Shares the host rustc's copies of the compiler crates.
#[allow(unused_extern_crates)]
extern crate rustc_driver;

mod convert;

use std::any::Any;
use std::fs;

use rustc_codegen_ssa::traits::CodegenBackend;
use rustc_codegen_ssa::{CodegenResults, CompiledModule, CrateInfo, ModuleKind};
use rustc_data_structures::fx::FxIndexMap;
use rustc_middle::dep_graph::{WorkProduct, WorkProductId};
use rustc_middle::mir::mono::{MonoItem, Visibility};
use rustc_middle::ty::{Instance, TyCtxt};
use rustc_session::Session;
use rustc_session::config::{CrateType, OutputFilenames, OutputType};
use rustc_target::spec::PanicStrategy;

use kfrc_x86::{FunctionCode, SymbolBinding};

pub struct KfrcCodegenBackend;

struct OngoingCodegen {
    modules: Vec<CompiledModule>,
    crate_info: CrateInfo,
}

impl CodegenBackend for KfrcCodegenBackend {
    fn locale_resource(&self) -> &'static str {
        ""
    }

    fn name(&self) -> &'static str {
        "kfrc"
    }

    fn init(&self, sess: &Session) {
        let target = &sess.target;
        if target.llvm_target.as_ref() != "x86_64-unknown-linux-gnu" {
            sess.dcx().fatal(format!(
                "kfrc: only x86_64-unknown-linux-gnu is supported, not `{}`",
                target.llvm_target
            ));
        }
        if sess.panic_strategy() != PanicStrategy::Abort {
            sess.dcx().fatal("kfrc: the PoC compiles `-Cpanic=abort` crates only (no unwind tables)");
        }
    }

    /// Same answer as cg_clif for x86-64: the ABI needs x87 and SSE2.
    fn target_config(&self, _sess: &Session) -> rustc_codegen_ssa::TargetConfig {
        let features: Vec<rustc_span::Symbol> = ["fxsr", "sse", "sse2", "x87"]
            .into_iter()
            .map(rustc_span::Symbol::intern)
            .collect();
        rustc_codegen_ssa::TargetConfig {
            target_features: features.clone(),
            unstable_target_features: features,
            has_reliable_f16: true,
            has_reliable_f16_math: true,
            has_reliable_f128: true,
            has_reliable_f128_math: true,
        }
    }

    fn codegen_crate<'tcx>(&self, tcx: TyCtxt<'tcx>) -> Box<dyn Any> {
        if tcx.crate_types().contains(&CrateType::Executable) && tcx.entry_fn(()).is_some() {
            tcx.dcx().fatal("kfrc: executables need the `main` entry shim, which is not implemented yet; build the program logic as a library");
        }
        if rustc_codegen_ssa::base::allocator_kind_for_codegen(tcx).is_some() {
            tcx.dcx().fatal("kfrc: the allocator shim is not implemented yet");
        }
        let outputs = tcx.output_filenames(());
        let invocation_temp = tcx.sess.invocation_temp.as_deref();
        let mut modules = Vec::new();
        for cgu in tcx.collect_and_partition_mono_items(()).codegen_units {
            let mut functions = Vec::new();
            for (item, data) in cgu.items_in_deterministic_order(tcx) {
                match item {
                    MonoItem::Fn(instance) => {
                        let function = compile_instance(tcx, instance);
                        let binding = match data.linkage {
                            rustc_hir::attrs::Linkage::Internal => SymbolBinding::Local,
                            _ => SymbolBinding::Global {
                                hidden: data.visibility == Visibility::Hidden,
                            },
                        };
                        functions.push(FunctionCode { function, binding });
                    }
                    MonoItem::Static(def_id) => tcx.dcx().fatal(format!(
                        "kfrc: statics are not supported yet (`{}`)",
                        tcx.def_path_str(def_id)
                    )),
                    MonoItem::GlobalAsm(_) => tcx.dcx().fatal("kfrc: global_asm! is not supported"),
                }
            }
            let bytes = kfrc_x86::write_object(&functions)
                .unwrap_or_else(|error| tcx.dcx().fatal(format!("kfrc: {error}")));
            let name = cgu.name().to_string();
            let path = outputs.temp_path_for_cgu(OutputType::Object, &name, invocation_temp);
            fs::write(&path, bytes)
                .unwrap_or_else(|error| tcx.dcx().fatal(format!("kfrc: writing {}: {error}", path.display())));
            modules.push(CompiledModule {
                name,
                kind: ModuleKind::Regular,
                object: Some(path),
                dwarf_object: None,
                bytecode: None,
                assembly: None,
                llvm_ir: None,
                links_from_incr_cache: Vec::new(),
            });
        }
        let target_cpu = tcx.sess.opts.cg.target_cpu.clone().unwrap_or_else(|| tcx.sess.target.cpu.to_string());
        Box::new(OngoingCodegen {
            modules,
            crate_info: CrateInfo::new(tcx, target_cpu),
        })
    }

    fn join_codegen(
        &self,
        ongoing_codegen: Box<dyn Any>,
        sess: &Session,
        _outputs: &OutputFilenames,
    ) -> (CodegenResults, FxIndexMap<WorkProductId, WorkProduct>) {
        let ongoing = ongoing_codegen.downcast::<OngoingCodegen>().expect("kfrc ongoing codegen");
        sess.dcx().abort_if_errors();
        (
            CodegenResults {
                modules: ongoing.modules,
                allocator_module: None,
                crate_info: ongoing.crate_info,
            },
            FxIndexMap::default(),
        )
    }
}

/// MIR → Body → R graph → verified → machine code, or a fatal error naming
/// the function and the unsupported construct.
fn compile_instance<'tcx>(tcx: TyCtxt<'tcx>, instance: Instance<'tcx>) -> kfrc_x86::MachineFunction {
    // Type names in dumps and errors use full paths: trimmed paths are only
    // allowed while rustc is emitting a diagnostic.
    rustc_middle::ty::print::with_no_trimmed_paths!(compile_instance_inner(tcx, instance))
}

fn compile_instance_inner<'tcx>(tcx: TyCtxt<'tcx>, instance: Instance<'tcx>) -> kfrc_x86::MachineFunction {
    let symbol = tcx.symbol_name(instance).name.to_string();
    let fail = |stage: &str, error: String| -> ! {
        tcx.dcx().fatal(format!("kfrc: cannot compile `{instance}` ({symbol}): {stage}: {error}"))
    };
    let body = convert::convert_instance(tcx, instance).unwrap_or_else(|error| fail("MIR conversion", error));
    if std::env::var_os("KFRC_DUMP_BODY").is_some() {
        eprintln!("{body}");
    }
    let mut graph = kfrc_son::build_graph(&body).unwrap_or_else(|error| fail("SoN construction", error.to_string()));
    kfrc_son::verify_graph(&mut graph).unwrap_or_else(|error| fail("verification", format!("{error}\n{}", graph.dump())));
    if std::env::var_os("KFRC_DUMP_GRAPH").is_some() {
        eprintln!("{}", graph.dump());
    }
    // `KFRC_INTERPRET=<symbol substring>=<arg>,<arg>`: run the reference
    // interpreter on the verified graph to separate graph bugs from
    // lowering bugs.
    if let Ok(request) = std::env::var("KFRC_INTERPRET")
        && let Some((pattern, arguments)) = request.split_once('=')
        && symbol.contains(pattern)
    {
        let arguments: Vec<u64> = arguments
            .split(',')
            .filter(|text| !text.is_empty())
            .map(|text| text.trim().parse::<i128>().map(|value| value as u64).unwrap_or(0))
            .collect();
        let mut no_calls = |callee: &str, _: &[u64]| Err(kfrc_son::InterpretError::UnknownCallee(callee.to_owned()));
        let outcome = kfrc_son::Interpreter::new(&graph).run(&arguments, &mut no_calls);
        eprintln!("kfrc interpret {symbol}{arguments:?} = {outcome:?}");
    }
    kfrc_x86::lower_function(&graph).unwrap_or_else(|error| fail("x86 lowering", error.to_string()))
}

/// Entry point looked up by rustc for `-Zcodegen-backend=<path>`.
#[unsafe(no_mangle)]
pub fn __rustc_codegen_backend() -> Box<dyn CodegenBackend> {
    Box::new(KfrcCodegenBackend)
}
