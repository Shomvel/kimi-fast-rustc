//! `rustc_codegen_kfrc`: the PoC shell (`docs/plan/poc/architecture.md`).
//!
//! rustc 1.94.1 runs its whole front end and mono-item collection; this
//! backend prepares polymorphic R once per eligible definition and specializes
//! it on demand. Other constructs enter the same semantic IR through a concrete
//! Typed Body. Rust values survive until explicit layout lowering.
//! It verifies the graph, lowers it with `kfrc-x86`, and hands one
//! object per codegen unit back to rustc's linker driver. Only this crate
//! links `rustc_*`; `kfrc-body`, `kfrc-son` and `kfrc-x86` do not.
//!
//! Anything outside the implemented subset is a fatal error naming the
//! function and construct — never a silent fallback to LLVM. Crates that
//! need unsupported features are built with the LLVM backend instead and
//! marked as such (`docs/plan/poc/scope.md`).

#![feature(rustc_private)]

extern crate rustc_abi;
extern crate rustc_ast;
extern crate rustc_codegen_ssa;
extern crate rustc_data_structures;
extern crate rustc_hir;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;
extern crate rustc_symbol_mangling;
extern crate rustc_target;

// Shares the host rustc's copies of the compiler crates.
#[allow(unused_extern_crates)]
extern crate rustc_driver;

mod abi;
mod allocator;
mod codegen;
mod constants;
mod convert;
mod definitions;
mod emission;
mod enum_layout;
mod inline_cost;
mod parallel;
mod permissions;
mod profile;
mod shell_entry;
mod type_identity;

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
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

use codegen::CrateGraphs;
use kfrc_x86::SymbolBinding;

pub struct KfrcCodegenBackend;

struct OngoingCodegen {
    modules: Vec<CompiledModule>,
    allocator_module: Option<CompiledModule>,
    crate_info: CrateInfo,
    measurement: Option<profile::TotalMeasurement>,
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
            sess.dcx()
                .fatal("kfrc: the PoC compiles `-Cpanic=abort` crates only (no unwind tables)");
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
        let threads = parallel::thread_limit().unwrap_or_else(|error| tcx.dcx().fatal(error));
        let measurement =
            profile::start_total(tcx.crate_name(rustc_hir::def_id::LOCAL_CRATE).as_str());
        let outputs = tcx.output_filenames(());
        let invocation_temp = tcx.sess.invocation_temp.as_deref();
        let mut modules = Vec::new();
        let entry_plan =
            rustc_middle::ty::print::with_no_trimmed_paths!(shell_entry::UnitEntryPlan::new(tcx));
        // Collection performs rustc's required-constant/mentioned-item checks
        // before any optimization. Emission below may omit internal functions
        // only after optimized machine/data references prove them unreachable.
        let partition = tcx.collect_and_partition_mono_items(());
        let mut graphs = CrateGraphs::default();
        let mut graph_jobs = Vec::new();
        let mut graph_instances = Vec::new();
        let mut graph_symbols = BTreeSet::new();
        let mut definitions = definitions::Definitions::default();
        let mut permission_totals = permissions::PermissionTotals::default();
        let mut adapters = BTreeMap::new();
        for cgu in partition.codegen_units {
            for (item, _) in cgu.items_in_deterministic_order(tcx) {
                let MonoItem::Fn(instance) = item else {
                    continue;
                };
                let symbol = tcx.symbol_name(instance).name.to_string();
                if graph_symbols.contains(&symbol) || adapters.contains_key(&symbol) {
                    continue;
                }
                if let Some(adapter) = entry_plan.as_ref().and_then(|plan| {
                    rustc_middle::ty::print::with_no_trimmed_paths!(plan.compile(tcx, instance))
                }) {
                    adapters.insert(symbol, adapter);
                    continue;
                }
                if matches!(
                    tcx.codegen_fn_attrs(instance.def_id()).inline,
                    rustc_hir::attrs::InlineAttr::Never
                ) {
                    graphs.no_inline.insert(symbol.clone());
                }
                let graph = rustc_middle::ty::print::with_no_trimmed_paths!(
                    prepare_instance_input(tcx, instance, &mut definitions, &mut permission_totals)
                );
                graph_symbols.insert(symbol);
                graph_instances.push(instance);
                graph_jobs.push(graph);
            }
        }
        for (instance, result) in graph_instances.into_iter().zip(parallel::map(
            graph_jobs,
            threads,
            codegen::prepare_graph,
        )) {
            let graph = result.unwrap_or_else(|error| {
                rustc_middle::ty::print::with_no_trimmed_paths!(tcx.dcx().fatal(format!(
                    "kfrc: cannot compile `{instance}` ({}): {}",
                    error.symbol, error.message
                )))
            });
            graphs.bodies.insert(graph.symbol.clone(), graph);
        }
        definitions.report();
        permission_totals.report(tcx.crate_name(rustc_hir::def_id::LOCAL_CRATE).as_str());
        for cgu in partition.codegen_units {
            // rustc may internalize a function during CGU partitioning even
            // when the source declaration has no explicit linkage attribute.
            for (item, data) in cgu.items_in_deterministic_order(tcx) {
                if data.linkage == rustc_hir::attrs::Linkage::Internal {
                    let instance = match item {
                        MonoItem::Fn(instance) => instance,
                        MonoItem::Static(def_id) => Instance::mono(tcx, def_id),
                        MonoItem::GlobalAsm(_) => continue,
                    };
                    graphs
                        .internal
                        .insert(tcx.symbol_name(instance).name.to_string());
                }
            }
        }
        graphs.collect_references();
        let mut jobs = Vec::new();
        for cgu in partition.codegen_units {
            let available: BTreeSet<String> = cgu
                .items_in_deterministic_order(tcx)
                .into_iter()
                .filter_map(|(item, _)| match item {
                    MonoItem::Fn(instance) => Some(tcx.symbol_name(instance).name.to_string()),
                    MonoItem::Static(def_id) => Some(
                        tcx.symbol_name(Instance::mono(tcx, def_id))
                            .name
                            .to_string(),
                    ),
                    _ => None,
                })
                .collect();
            let mut data_objects: Vec<kfrc_body::DataObject> = Vec::new();
            let mut pending_functions = BTreeMap::new();
            let mut needed = BTreeSet::new();
            if let Some((entry, _)) = tcx.entry_fn(()) {
                needed.insert(tcx.symbol_name(Instance::mono(tcx, entry)).name.to_string());
            }
            for (item, data) in cgu.items_in_deterministic_order(tcx) {
                match item {
                    MonoItem::Fn(instance) => {
                        let symbol = tcx.symbol_name(instance).name.to_string();
                        let binding = match data.linkage {
                            rustc_hir::attrs::Linkage::Internal => SymbolBinding::Local,
                            _ => SymbolBinding::Global {
                                hidden: data.visibility == Visibility::Hidden,
                            },
                        };
                        // Global symbols may be referenced from other CGUs or
                        // downstream crates. Runtime adapters are shell roots.
                        if data.linkage != rustc_hir::attrs::Linkage::Internal
                            || adapters.contains_key(&symbol)
                            || std::env::var("KFRC_OPTIMIZE").as_deref() == Ok("0")
                        {
                            needed.insert(symbol.clone());
                        }
                        pending_functions.insert(symbol, binding);
                    }
                    MonoItem::Static(def_id) => {
                        let binding = match (data.linkage, data.visibility) {
                            (rustc_hir::attrs::Linkage::Internal, _) => {
                                kfrc_body::DataBinding::Local
                            }
                            (rustc_hir::attrs::Linkage::External, Visibility::Hidden) => {
                                kfrc_body::DataBinding::Global { hidden: true }
                            }
                            (rustc_hir::attrs::Linkage::External, Visibility::Default) => {
                                kfrc_body::DataBinding::Global { hidden: false }
                            }
                            (linkage, visibility) => tcx.dcx().fatal(format!(
                                "kfrc: static `{}` has unsupported linkage {linkage:?} and visibility {visibility:?}",
                                tcx.def_path_str(def_id)
                            )),
                        };
                        let symbol = tcx.symbol_name(Instance::mono(tcx, def_id)).name;
                        let mut builder = kfrc_body::BodyBuilder::new(
                            symbol,
                            kfrc_body::FunctionAbi {
                                args: Vec::new(),
                                ret: kfrc_body::PassMode::Ignore,
                                c_variadic: false,
                            },
                            kfrc_body::RecordedTypes::new(),
                        );
                        constants::ConstantCx::new(tcx, symbol)
                            .static_data(def_id, binding, &mut builder)
                            .unwrap_or_else(|error| {
                                tcx.dcx().fatal(format!("kfrc: static data: {error}"))
                            });
                        merge_data_objects(tcx, &mut data_objects, builder.finish().data_objects);
                    }
                    MonoItem::GlobalAsm(_) => tcx.dcx().fatal("kfrc: global_asm! is not supported"),
                }
            }
            jobs.push(emission::CguJob {
                available,
                pending_functions,
                needed,
                data_objects,
            });
        }
        let compiled = emission::compile(jobs, threads, &graphs, &adapters);
        for (cgu, result) in partition.codegen_units.iter().zip(compiled) {
            let mut compiled = result.unwrap_or_else(|error| {
                let instance =
                    cgu.items_in_deterministic_order(tcx)
                        .into_iter()
                        .find_map(|(item, _)| {
                            if let MonoItem::Fn(instance) = item
                                && tcx.symbol_name(instance).name == error.symbol
                            {
                                Some(instance)
                            } else {
                                None
                            }
                        });
                rustc_middle::ty::print::with_no_trimmed_paths!({
                    let context = instance
                        .map(|instance| format!("`{instance}` ({})", error.symbol))
                        .unwrap_or(error.symbol);
                    tcx.dcx()
                        .fatal(format!("kfrc: cannot compile {context}: {}", error.message))
                })
            });
            if std::env::var_os("KFRC_SPECIALIZATION_STATS").is_some() {
                eprintln!(
                    "kfrc emission {}: functions={} internal_omitted={}",
                    cgu.name(),
                    compiled.functions.len(),
                    compiled.internal_omitted
                );
            }
            let functions = &mut compiled.functions;
            let data_objects = &compiled.data_objects;
            if tcx.crate_types().contains(&CrateType::Executable) {
                if let Some(wrapper) = shell_entry::entry_wrapper(tcx, cgu.is_primary(), functions)
                {
                    functions.push(wrapper);
                }
            }
            let bytes = profile::measure("object", cgu.name().as_str(), || {
                kfrc_x86::write_object_with_data(functions, data_objects)
            })
            .unwrap_or_else(|error| tcx.dcx().fatal(format!("kfrc: {error}")));
            let name = cgu.name().to_string();
            let path = outputs.temp_path_for_cgu(OutputType::Object, &name, invocation_temp);
            fs::write(&path, bytes).unwrap_or_else(|error| {
                tcx.dcx()
                    .fatal(format!("kfrc: writing {}: {error}", path.display()))
            });
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
        let target_cpu = tcx
            .sess
            .opts
            .cg
            .target_cpu
            .clone()
            .unwrap_or_else(|| tcx.sess.target.cpu.to_string());
        Box::new(OngoingCodegen {
            modules,
            allocator_module: allocator::codegen(tcx),
            crate_info: CrateInfo::new(tcx, target_cpu),
            measurement,
        })
    }

    fn join_codegen(
        &self,
        ongoing_codegen: Box<dyn Any>,
        sess: &Session,
        _outputs: &OutputFilenames,
    ) -> (CodegenResults, FxIndexMap<WorkProductId, WorkProduct>) {
        let ongoing = ongoing_codegen
            .downcast::<OngoingCodegen>()
            .expect("kfrc ongoing codegen");
        sess.dcx().abort_if_errors();
        let result = (
            CodegenResults {
                modules: ongoing.modules,
                allocator_module: ongoing.allocator_module,
                crate_info: ongoing.crate_info,
            },
            FxIndexMap::default(),
        );
        if let Some(measurement) = ongoing.measurement {
            measurement.finish();
        }
        result
    }
}

fn merge_data_objects(
    tcx: TyCtxt<'_>,
    into: &mut Vec<kfrc_body::DataObject>,
    objects: Vec<kfrc_body::DataObject>,
) {
    for object in objects {
        if let Some(existing) = into
            .iter()
            .find(|existing| existing.symbol == object.symbol)
        {
            if existing != &object {
                tcx.dcx().fatal(format!(
                    "kfrc: conflicting definitions of data `{}`",
                    object.symbol
                ));
            }
        } else {
            into.push(object);
        }
    }
}

fn prepare_instance_input<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: Instance<'tcx>,
    definitions: &mut definitions::Definitions<'tcx>,
    permission_totals: &mut permissions::PermissionTotals,
) -> codegen::GraphJob {
    let symbol = tcx.symbol_name(instance).name.to_string();
    let fail = |stage: &str, error: String| -> ! {
        tcx.dcx().fatal(format!(
            "kfrc: cannot compile `{instance}` ({symbol}): {stage}: {error}"
        ))
    };
    let input = match definitions.specialize(tcx, instance) {
        Ok(Some(specialized)) => codegen::GraphInput::Specialized {
            graph: specialized.graph,
            dirty_nodes: specialized.dirty_nodes,
            local_slots: specialized.local_slots,
        },
        Ok(None) => {
            let body = convert::convert_instance(tcx, instance)
                .unwrap_or_else(|error| fail("MIR conversion", error));
            if std::env::var_os("KFRC_DUMP_BODY").is_some() {
                eprintln!("{body}");
            }
            codegen::GraphInput::Body(body)
        }
        Err(error) => fail("definition specialization", error),
    };
    // RS5.1: R carries no permissions; the S instance receives them here,
    // before its first optimization, from the instance's own rustc types.
    let permissions =
        if permissions::enabled().unwrap_or_else(|error| fail("argument permissions", error)) {
            let (arguments, calls) = profile::measure("permissions_s", &symbol, || {
                (
                    permissions::definition_permissions(tcx, instance, permission_totals),
                    permissions::callee_permissions(tcx, instance),
                )
            });
            Some(codegen::Permissions {
                arguments: arguments.unwrap_or_else(|error| fail("argument permissions", error)),
                calls: calls.unwrap_or_else(|error| fail("call permissions", error)),
            })
        } else {
            None
        };
    codegen::GraphJob {
        symbol,
        input,
        permissions,
    }
}

/// Entry point looked up by rustc for `-Zcodegen-backend=<path>`.
#[unsafe(no_mangle)]
pub fn __rustc_codegen_backend() -> Box<dyn CodegenBackend> {
    Box::new(KfrcCodegenBackend)
}
