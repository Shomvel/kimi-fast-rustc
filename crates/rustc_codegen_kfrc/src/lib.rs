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
mod constants;
mod convert;
mod definitions;
mod emission;
mod enum_layout;
mod inline_cost;
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

use kfrc_x86::{FunctionCode, SymbolBinding};

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
        let mut definitions = definitions::Definitions::default();
        let mut permission_totals = permissions::PermissionTotals::default();
        let mut adapters = BTreeMap::new();
        for cgu in partition.codegen_units {
            for (item, _) in cgu.items_in_deterministic_order(tcx) {
                let MonoItem::Fn(instance) = item else {
                    continue;
                };
                let symbol = tcx.symbol_name(instance).name.to_string();
                if graphs.bodies.contains_key(&symbol) || adapters.contains_key(&symbol) {
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
                let graph = rustc_middle::ty::print::with_no_trimmed_paths!(build_instance_graph(
                    tcx,
                    instance,
                    &mut definitions,
                    &mut permission_totals
                ));
                graphs.bodies.insert(symbol, graph);
            }
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
            let provider = CguGraphs {
                all: &graphs,
                available: &available,
            };
            let mut functions = Vec::new();
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
                        pending_functions.insert(symbol, (instance, binding));
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
            emission::data_references(&data_objects, &mut needed);
            while let Some(symbol) = needed.pop_first() {
                let Some((instance, binding)) = pending_functions.remove(&symbol) else {
                    continue;
                };
                let (function, objects) = if let Some(adapter) = adapters.get(&symbol) {
                    adapter.clone()
                } else {
                    rustc_middle::ty::print::with_no_trimmed_paths!(compile_instance_graph(
                        tcx,
                        instance,
                        graphs.bodies[&symbol].clone(),
                        &provider
                    ))
                };
                emission::function_references(&function, &mut needed);
                emission::data_references(&objects, &mut needed);
                merge_data_objects(tcx, &mut data_objects, objects);
                functions.push(FunctionCode { function, binding });
            }
            if std::env::var_os("KFRC_SPECIALIZATION_STATS").is_some() {
                eprintln!(
                    "kfrc emission {}: functions={} internal_omitted={}",
                    cgu.name(),
                    functions.len(),
                    pending_functions.len()
                );
            }
            if tcx.crate_types().contains(&CrateType::Executable) {
                if let Some(wrapper) = shell_entry::entry_wrapper(tcx, cgu.is_primary(), &functions)
                {
                    functions.push(wrapper);
                }
            }
            let bytes = profile::measure("object", cgu.name().as_str(), || {
                kfrc_x86::write_object_with_data(&functions, &data_objects)
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

#[derive(Default)]
struct CrateGraphs {
    bodies: BTreeMap<String, kfrc_son::FunctionGraph>,
    no_inline: BTreeSet<String>,
    internal: BTreeSet<String>,
    references: BTreeMap<String, BTreeSet<String>>,
    /// Direct call sites per callee, by calling function.
    call_sites: BTreeMap<String, BTreeMap<String, usize>>,
    /// Symbols referenced other than by a direct call.
    address_taken: BTreeSet<String>,
}

impl CrateGraphs {
    fn collect_references(&mut self) {
        for (symbol, graph) in &self.bodies {
            let mut references = BTreeSet::new();
            for node in graph.ids() {
                match graph.op(node) {
                    kfrc_son::Opcode::Call => {
                        let callee = &graph.callees[graph.payload(node) as usize].symbol;
                        references.insert(callee.clone());
                        *self
                            .call_sites
                            .entry(callee.clone())
                            .or_default()
                            .entry(symbol.clone())
                            .or_default() += 1;
                    }
                    kfrc_son::Opcode::SymbolAddress => {
                        let target = &graph.symbols[graph.payload(node) as usize].0;
                        references.insert(target.clone());
                        self.address_taken.insert(target.clone());
                    }
                    _ => {}
                }
            }
            for object in &graph.data_objects {
                for relocation in &object.relocations {
                    references.insert(relocation.target.clone());
                    self.address_taken.insert(relocation.target.clone());
                }
            }
            self.references.insert(symbol.clone(), references);
        }
    }
}

/// Inlining must not create relocations to another object's local functions.
/// Pure bodies remain available across CGUs; private symbol dependencies do not.
struct CguGraphs<'a> {
    all: &'a CrateGraphs,
    available: &'a BTreeSet<String>,
}

impl kfrc_son::GraphProvider for CguGraphs<'_> {
    fn graph(&self, symbol: &str) -> Option<&kfrc_son::FunctionGraph> {
        if self.all.no_inline.contains(symbol)
            || self.all.references.get(symbol).is_some_and(|references| {
                references.iter().any(|reference| {
                    self.all.internal.contains(reference) && !self.available.contains(reference)
                })
            })
        {
            None
        } else {
            self.all.bodies.get(symbol)
        }
    }

    // Each CGU owns its copy of an internal function, so count its callers there.
    fn sole_call_site(&self, symbol: &str) -> bool {
        self.all.internal.contains(symbol)
            && !self.all.address_taken.contains(symbol)
            && self.all.call_sites.get(symbol).is_some_and(|callers| {
                callers
                    .iter()
                    .filter(|(caller, _)| self.available.contains(*caller))
                    .map(|(_, count)| count)
                    .sum::<usize>()
                    == 1
            })
    }
}

fn build_instance_graph<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: Instance<'tcx>,
    definitions: &mut definitions::Definitions<'tcx>,
    permission_totals: &mut permissions::PermissionTotals,
) -> kfrc_son::FunctionGraph {
    let symbol = tcx.symbol_name(instance).name.to_string();
    let fail = |stage: &str, error: String| -> ! {
        tcx.dcx().fatal(format!(
            "kfrc: cannot compile `{instance}` ({symbol}): {stage}: {error}"
        ))
    };
    let (mut graph, specialization_seeds) = match definitions.specialize(tcx, instance) {
        Ok(Some(specialized)) => (
            specialized.graph,
            Some((specialized.dirty_nodes, specialized.local_slots)),
        ),
        Ok(None) => {
            let body = convert::convert_instance(tcx, instance)
                .unwrap_or_else(|error| fail("MIR conversion", error));
            if std::env::var_os("KFRC_DUMP_BODY").is_some() {
                eprintln!("{body}");
            }
            let graph = profile::measure("build", &symbol, || kfrc_son::build_graph(&body))
                .unwrap_or_else(|error| fail("SoN construction", error.to_string()));
            (graph, None)
        }
        Err(error) => fail("definition specialization", error),
    };
    // RS5.1: R carries no permissions; the S instance receives them here,
    // before its first optimization, from the instance's own rustc types.
    if permissions::enabled().unwrap_or_else(|error| fail("argument permissions", error)) {
        let (arguments, calls) = profile::measure("permissions_s", &symbol, || {
            (
                permissions::definition_permissions(tcx, instance, permission_totals),
                permissions::callee_permissions(tcx, instance),
            )
        });
        graph.arg_permissions =
            arguments.unwrap_or_else(|error| fail("argument permissions", error));
        graph.callee_permissions = calls
            .unwrap_or_else(|error| fail("call permissions", error))
            .into_iter()
            .filter(|(symbol, _)| graph.callees.iter().any(|target| &target.symbol == symbol))
            .collect();
    }
    profile::measure("verify_s_input", &symbol, || {
        kfrc_son::verify_graph(&mut graph)
    })
    .unwrap_or_else(|error| {
        fail(
            "semantic verification",
            format!("{error}\n{}", graph.dump()),
        )
    });
    if std::env::var_os("KFRC_DUMP_GRAPH").is_some() {
        eprintln!("kfrc semantic input {symbol}:\n{}", graph.dump());
    }
    // R has already optimized the unchanged definition. Only substitutions,
    // pruned control and type-dependent users seed its instance's scalar work.
    // Concrete Body input has no prepared R and needs the full S pipeline.
    // Both paths cache their simplified source once for all inline consumers.
    if std::env::var("KFRC_OPTIMIZE").as_deref() != Ok("0") {
        let stats = profile::measure("optimize_s", &symbol, || match specialization_seeds {
            Some((dirty_nodes, local_slots)) => {
                kfrc_son::optimize_specialized_seams(&mut graph, &dirty_nodes, &local_slots)
            }
            None => kfrc_son::optimize_specialized_graph(&mut graph),
        })
        .unwrap_or_else(|error| fail("semantic optimization", error.to_string()));
        if std::env::var_os("KFRC_DUMP_GRAPH").is_some() {
            eprintln!("kfrc optimization {symbol}: {stats:?}");
        }
    }
    graph
}

fn compile_instance_graph<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: Instance<'tcx>,
    mut graph: kfrc_son::FunctionGraph,
    provider: &CguGraphs<'_>,
) -> (kfrc_x86::MachineFunction, Vec<kfrc_body::DataObject>) {
    let symbol = graph.symbol.clone();
    let fail = |stage: &str, error: String| -> ! {
        tcx.dcx().fatal(format!(
            "kfrc: cannot compile `{instance}` ({symbol}): {stage}: {error}"
        ))
    };
    if std::env::var("KFRC_OPTIMIZE").as_deref() != Ok("0") {
        let identities = profile::measure("identity_s", &symbol, || {
            kfrc_son::fold_identity_calls(&mut graph, provider, None)
        })
        .unwrap_or_else(|error| fail("identity call simplification", error.to_string()));
        let mut changed = identities.calls_folded > 0;
        let mut dirty_nodes = identities.dirty_nodes.clone();
        let mut argument_slots = identities.argument_slots.clone();
        if std::env::var_os("KFRC_DUMP_GRAPH").is_some() {
            eprintln!("kfrc identity simplification {symbol}: {identities:?}");
        }
        if std::env::var("KFRC_INLINE").as_deref() != Ok("0") {
            // Inlining retains node identities. Only its new call sites need
            // another identity-summary query; pre-existing calls were checked.
            let before_inline = graph.len();
            let cost_policy = inline_cost::policy()
                .unwrap_or_else(|error| fail("inline cost configuration", error));
            let inlined = profile::measure("inline_s", &symbol, || {
                kfrc_son::inline_calls_with_cost(
                    &mut graph, provider, kfrc_son::InlineBudget::default(),
                    cost_policy, &kfrc_son::analysis::cost::GenericCostModel,
                )
            })
            .unwrap_or_else(|error| fail("semantic inlining", error.to_string()));
            inline_cost::report(&symbol, &inlined);
            if inlined.calls_inlined > 0 {
                changed = true;
                dirty_nodes.extend_from_slice(&inlined.dirty_nodes);
                argument_slots.extend_from_slice(&inlined.argument_slots);
                let new_calls: Vec<_> = (before_inline..graph.len())
                    .map(|index| kfrc_son::NodeId(index as u32))
                    .filter(|node| {
                        !graph.is_dead(*node) && graph.op(*node) == kfrc_son::Opcode::Call
                    })
                    .collect();
                if !new_calls.is_empty() {
                    let nested = profile::measure("post_inline_identity_s", &symbol, || {
                        kfrc_son::fold_identity_calls(&mut graph, provider, Some(&new_calls))
                    })
                    .unwrap_or_else(|error| {
                        fail("nested identity simplification", error.to_string())
                    });
                    dirty_nodes.extend_from_slice(&nested.dirty_nodes);
                    argument_slots.extend_from_slice(&nested.argument_slots);
                    if std::env::var_os("KFRC_DUMP_GRAPH").is_some() {
                        eprintln!("kfrc nested identity simplification {symbol}: {nested:?}");
                    }
                }
            }
            if std::env::var_os("KFRC_DUMP_GRAPH").is_some() {
                eprintln!("kfrc inlining {symbol}: {inlined:?}");
            }
        }
        if changed {
            let seams = profile::measure("post_inline_optimize_s", &symbol, || {
                kfrc_son::optimize_specialized_seams(&mut graph, &dirty_nodes, &argument_slots)
            })
            .unwrap_or_else(|error| fail("call seam optimization", error.to_string()));
            if std::env::var_os("KFRC_DUMP_GRAPH").is_some() {
                eprintln!("kfrc seam optimization {symbol}: {seams:?}");
            }
        }
    }
    profile::measure("verify_s_output", &symbol, || {
        kfrc_son::verify_graph(&mut graph)
    })
    .unwrap_or_else(|error| fail("verification", format!("{error}\n{}", graph.dump())));
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
            .map(|text| {
                text.trim()
                    .parse::<i128>()
                    .map(|value| value as u64)
                    .unwrap_or(0)
            })
            .collect();
        let values: Vec<_> = arguments
            .iter()
            .copied()
            .map(kfrc_son::Value::Scalar)
            .collect();
        let mut no_calls = |callee: &str, _: &[kfrc_son::Value]| {
            Err(kfrc_son::InterpretError::UnknownCallee(callee.to_owned()))
        };
        let outcome = kfrc_son::Interpreter::new(&graph).run_values(&values, &mut no_calls);
        eprintln!("kfrc semantic interpret {symbol}{arguments:?} = {outcome:?}");
    }
    let layout_stats = profile::measure("layout", &symbol, || kfrc_son::lower_layout(&mut graph))
        .unwrap_or_else(|error| fail("layout lowering", error.to_string()));
    if std::env::var_os("KFRC_DUMP_GRAPH").is_some()
        || std::env::var_os("KFRC_SPECIALIZATION_STATS").is_some()
    {
        eprintln!("kfrc layout statistics {symbol}: {layout_stats:?}");
    }
    profile::measure("verify_l", &symbol, || kfrc_son::verify_graph(&mut graph))
        .unwrap_or_else(|error| fail("layout verification", format!("{error}\n{}", graph.dump())));
    if std::env::var_os("KFRC_DUMP_GRAPH").is_some() {
        eprintln!("kfrc layout output {symbol}:\n{}", graph.dump());
    }
    let function = profile::measure("x86", &symbol, || kfrc_x86::lower_function(&graph))
        .unwrap_or_else(|error| fail("x86 lowering", error.to_string()));
    (function, graph.data_objects)
}

/// Entry point looked up by rustc for `-Zcodegen-backend=<path>`.
#[unsafe(no_mangle)]
pub fn __rustc_codegen_backend() -> Box<dyn CodegenBackend> {
    Box::new(KfrcCodegenBackend)
}
