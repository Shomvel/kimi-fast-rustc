//! Owned graph compilation. No rustc context crosses the worker boundary.
use crate::{inline_cost, profile};
use std::collections::{BTreeMap, BTreeSet};

mod argument_promotion;
mod cgscc;
mod conditional_returns;
mod wrappers;

#[derive(Debug)]
pub(crate) struct CompileError {
    pub symbol: String,
    pub message: String,
}

pub(crate) struct EmissionStart<'a> {
    pub(crate) graph: &'a kfrc_son::FunctionGraph,
    pub(crate) inlining_complete: bool,
}

#[derive(Default)]
pub(crate) struct CrateGraphs {
    /// Data definitions are shared by every graph and lowered only at emission.
    pub(crate) data: crate::program_data::ProgramData,
    /// Shared sources used to price and inline callees in every CGU.
    pub(crate) bodies: BTreeMap<String, kfrc_son::FunctionGraph>,
    pub(crate) no_inline: BTreeSet<String>,
    pub(crate) always_inline: BTreeSet<String>,
    pub(crate) internal: BTreeSet<String>,
    /// SCC preparation already exhausted context-independent inline work.
    pub(crate) inlining_complete: BTreeSet<String>,
    /// One signature-stable crate snapshot, consumed once after local inlining.
    pub(crate) conditional_returns: kfrc_son::ConditionalReturns,
    pub(crate) references: BTreeMap<String, BTreeSet<String>>,
    /// Direct call sites per callee, by calling function.
    pub(crate) call_sites: BTreeMap<String, BTreeMap<String, usize>>,
    /// Symbols referenced other than by a direct call.
    pub(crate) address_taken: BTreeSet<String>,
}

impl CrateGraphs {
    pub(crate) fn lift_data(
        &mut self,
        jobs: &mut [crate::emission::CguJob],
        adapters: &mut BTreeMap<String, crate::machine::CompiledFunction>,
    ) -> Result<(), CompileError> {
        let mut owners = BTreeMap::new();
        for (cgu, job) in jobs.iter_mut().enumerate() {
            for symbol in job.pending_functions.keys() {
                owners.entry(symbol.clone()).or_insert(cgu);
            }
            for object in std::mem::take(&mut job.data_objects) {
                // Named statics selected by rustc retain their definitions.
                // Their anonymous allocation dependencies are reached by relocation.
                if !object.unnamed_address || object.binding != kfrc_body::DataBinding::Local {
                    job.needed.insert(object.symbol.clone());
                }
                self.data.insert(object, cgu)?;
            }
        }
        for (symbol, graph) in &mut self.bodies {
            let owner = owners[symbol];
            for object in std::mem::take(&mut graph.data_objects) {
                if object.binding != kfrc_body::DataBinding::Local {
                    jobs[owner].needed.insert(object.symbol.clone());
                }
                self.data.insert(object, owner)?;
            }
        }
        for (symbol, (_, objects)) in adapters.iter_mut() {
            let owner = owners[symbol];
            for object in std::mem::take(objects) {
                if object.binding != kfrc_body::DataBinding::Local {
                    jobs[owner].needed.insert(object.symbol.clone());
                }
                self.data.insert(object, owner)?;
            }
        }
        let aliases = self.data.canonicalize();
        let definitions = std::sync::Arc::new(self.data.objects.clone());
        for graph in self.bodies.values_mut() {
            crate::program_data::rewrite_graph(graph, &aliases);
            graph.program_data = definitions.clone();
        }
        for (function, _) in adapters.values_mut() {
            crate::program_data::rewrite_code(function, &aliases);
        }
        for job in jobs {
            crate::program_data::rewrite_names(&mut job.needed, &aliases);
        }
        self.collect_references();
        Ok(())
    }

    /// Rebuild all proofs after crate-level signature transforms. Callers keep
    /// only the attributes of their current direct targets, guarded by ABI.
    pub(crate) fn infer_function_attrs(&mut self) {
        for graph in self.bodies.values_mut() {
            graph.callee_attrs.clear();
        }
        if std::env::var("KFRC_OPTIMIZE").as_deref() == Ok("0")
            || std::env::var("KFRC_FUNCTION_ATTRS").as_deref() == Ok("0")
        {
            return;
        }
        let attrs = kfrc_son::analysis::function_attrs::infer_exact(&self.bodies, &self.internal);
        if std::env::var_os("KFRC_SPECIALIZATION_STATS").is_some()
            || std::env::var_os("KFRC_DUMP_GRAPH").is_some()
        {
            use kfrc_son::analysis::function_attrs::MemoryEffects;
            let exact: Vec<_> = attrs
                .iter()
                .filter(|(symbol, _)| self.internal.contains(*symbol))
                .map(|(_, attrs)| attrs)
                .collect();
            eprintln!(
                "kfrc function attrs: exact={} none={} readonly={} nocapture={} noreturn={} willreturn={}",
                exact.len(),
                exact
                    .iter()
                    .filter(|attrs| attrs.memory == MemoryEffects::None)
                    .count(),
                exact
                    .iter()
                    .filter(|attrs| attrs.memory == MemoryEffects::ReadOnly)
                    .count(),
                exact
                    .iter()
                    .flat_map(|attrs| &attrs.arguments)
                    .filter(|attrs| attrs.nocapture)
                    .count(),
                exact.iter().filter(|attrs| attrs.noreturn).count(),
                exact.iter().filter(|attrs| attrs.willreturn).count(),
            );
        }
        for graph in self.bodies.values_mut() {
            for target in &graph.callees {
                if let Some(attrs) = attrs.get(&target.symbol)
                    && attrs.abi == target.abi
                {
                    graph
                        .callee_attrs
                        .insert(target.symbol.clone(), attrs.clone());
                }
            }
        }
    }

    pub(crate) fn apply_function_attrs(&mut self) -> Result<(), CompileError> {
        let mut loads = 0;
        let mut calls = 0;
        let mut noreturn = 0;
        for (symbol, graph) in self.bodies.iter_mut() {
            if !graph.ids().any(|node| {
                graph.op(node) == kfrc_son::Opcode::Call
                    && graph
                        .callee_attrs(graph.payload(node))
                        .is_some_and(|attrs| attrs.memory.cannot_write() || attrs.noreturn)
            }) {
                continue;
            }
            let stats = kfrc_son::optimize_call_effects(graph).map_err(|error| CompileError {
                symbol: symbol.clone(),
                message: format!("call effects: {error}"),
            })?;
            loads += stats.loads_forwarded;
            calls += stats.calls_crossed;
            noreturn += stats.noreturn_calls;
        }
        if std::env::var_os("KFRC_SPECIALIZATION_STATS").is_some()
            || std::env::var_os("KFRC_DUMP_GRAPH").is_some()
        {
            eprintln!(
                "kfrc call effects: loads_forwarded={loads} calls_crossed={calls} noreturn={noreturn}"
            );
        }
        Ok(())
    }

    pub(crate) fn collect_references(&mut self) {
        self.references.clear();
        self.call_sites.clear();
        self.address_taken.clear();
        for (symbol, graph) in &self.bodies {
            let mut references = BTreeSet::new();
            let mut calls = BTreeMap::<String, usize>::new();
            for node in graph.ids() {
                match graph.op(node) {
                    kfrc_son::Opcode::Call => {
                        let callee = &graph.callees[graph.payload(node) as usize].symbol;
                        references.insert(callee.clone());
                        *calls.entry(callee.clone()).or_default() += 1;
                    }
                    kfrc_son::Opcode::SymbolAddress | kfrc_son::Opcode::ThreadLocalAddress => {
                        let target = &graph.symbols[graph.payload(node) as usize].0;
                        references.insert(target.clone());
                        self.address_taken.insert(target.clone());
                    }
                    _ => {}
                }
            }
            for partition in &graph.memory_partitions {
                if let kfrc_son::MemoryPartition::Const { symbol } = *partition {
                    let target = &graph.symbols[symbol as usize].0;
                    references.insert(target.clone());
                    self.address_taken.insert(target.clone());
                }
            }
            for object in &graph.data_objects {
                for relocation in &object.relocations {
                    references.insert(relocation.target.clone());
                    self.address_taken.insert(relocation.target.clone());
                }
            }
            self.data.expand_references(&mut references);
            // Data relocations can take addresses of internal functions too.
            for reference in &references {
                if self.data.objects.contains_key(reference) {
                    let mut indirect = BTreeSet::from([reference.clone()]);
                    self.data.expand_references(&mut indirect);
                    self.address_taken.extend(indirect);
                }
            }
            for (callee, count) in calls {
                let sites = self
                    .call_sites
                    .entry(callee)
                    .or_default()
                    .entry(symbol.clone())
                    .or_default();
                *sites = (*sites).max(count);
            }
            self.references.insert(symbol.clone(), references);
        }
    }
}

/// All crate definitions are visible during optimization. The emitter assigns
/// one owner and promotes a local binding when another CGU references it.
pub(crate) struct CrateProvider<'a> {
    pub(crate) all: &'a CrateGraphs,
}

impl<'a> CrateProvider<'a> {
    pub(crate) fn emission_graph(&self, symbol: &str) -> EmissionStart<'a> {
        EmissionStart {
            graph: &self.all.bodies[symbol],
            inlining_complete: self.all.inlining_complete.contains(symbol),
        }
    }
}

impl kfrc_son::GraphProvider for CrateProvider<'_> {
    fn always_inline(&self, symbol: &str) -> bool {
        self.all.always_inline.contains(symbol)
    }

    fn graph(&self, symbol: &str) -> Option<&kfrc_son::FunctionGraph> {
        if self.all.no_inline.contains(symbol) {
            None
        } else {
            self.all.bodies.get(symbol)
        }
    }

    fn preparation_graph(&self, symbol: &str) -> Option<&kfrc_son::FunctionGraph> {
        (!self.all.no_inline.contains(symbol))
            .then(|| self.all.bodies.get(symbol))
            .flatten()
    }

    // A function has one crate-wide owner, so count every caller of it.
    fn sole_call_site(&self, symbol: &str) -> bool {
        self.all.internal.contains(symbol)
            && !self.all.address_taken.contains(symbol)
            && self
                .all
                .call_sites
                .get(symbol)
                .is_some_and(|callers| callers.iter().map(|(_, count)| count).sum::<usize>() == 1)
    }
}

pub(crate) fn compile_function(
    mut graph: kfrc_son::FunctionGraph,
    provider: &CrateProvider<'_>,
    inlining_complete: bool,
) -> Result<crate::machine::CompiledFunction, CompileError> {
    let symbol = graph.symbol.clone();
    let fail = |stage: &str, error: String| CompileError {
        symbol: symbol.clone(),
        message: format!("{stage}: {error}"),
    };
    if std::env::var("KFRC_OPTIMIZE").as_deref() != Ok("0") {
        if !inlining_complete {
            let identities = profile::measure("identity_s", &symbol, || {
                kfrc_son::fold_identity_calls(&mut graph, provider, None)
            })
            .map_err(|error| fail("identity call simplification", error.to_string()))?;
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
                    .map_err(|error| fail("inline cost configuration", error))?;
                let limits = inline_cost::limits()
                    .map_err(|error| fail("inline limit configuration", error))?;
                let inlined = profile::measure("inline_s", &symbol, || {
                    kfrc_son::inline_calls_with_cost(
                        &mut graph,
                        provider,
                        kfrc_son::InlineBudget {
                            max_nodes: limits.invocation_nodes,
                            ..limits.budget
                        },
                        cost_policy,
                        inline_cost::model(cost_policy),
                    )
                })
                .map_err(|error| fail("semantic inlining", error.to_string()))?;
                inline_cost::report(&symbol, &inlined, cost_policy);
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
                        .map_err(|error| {
                            fail("nested identity simplification", error.to_string())
                        })?;
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
                .map_err(|error| fail("call seam optimization", error.to_string()))?;
                if std::env::var_os("KFRC_DUMP_GRAPH").is_some() {
                    eprintln!("kfrc seam optimization {symbol}: {seams:?}");
                }
            }
        }
        profile::measure("conditional_returns_s", &symbol, || {
            provider.all.simplify_conditional_returns(&mut graph)
        })?;
        if std::env::var("KFRC_TAIL_RECURSION").as_deref() != Ok("0") {
            let tails = profile::measure("tail_recursion_s", &symbol, || {
                kfrc_son::eliminate_tail_recursion(&mut graph)
            })
            .map_err(|error| fail("tail recursion elimination", error.to_string()))?;
            if std::env::var_os("KFRC_DUMP_GRAPH").is_some()
                || std::env::var_os("KFRC_SPECIALIZATION_STATS").is_some()
            {
                eprintln!("kfrc tail recursion {symbol}: {tails:?}");
            }
        }
    }
    profile::measure("verify_s_output", &symbol, || {
        kfrc_son::verify::prepare_pipeline_output(&mut graph)
    })
    .map_err(|error| fail("verification", format!("{error}\n{}", graph.dump())))?;
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
    let (lowered, layout_stats) = profile::measure("layout", &symbol, || {
        static CLONE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *CLONE.get_or_init(|| std::env::var("KFRC_LAYOUT_CLONE").as_deref() == Ok("1")) {
            let stats = kfrc_son::lower_layout(&mut graph)?;
            Ok((graph, stats))
        } else {
            // Emission already owns this graph and returns on failure.
            kfrc_son::lower_layout_owned(graph, kfrc_son::LayoutOptions::default())
        }
    })
    .map_err(|error: kfrc_son::LayoutError| fail("layout lowering", error.to_string()))?;
    graph = lowered;
    if std::env::var_os("KFRC_DUMP_GRAPH").is_some()
        || std::env::var_os("KFRC_SPECIALIZATION_STATS").is_some()
        || std::env::var_os("KFRC_LAYOUT_BYTE_LADDERS_STATS").is_some()
    {
        eprintln!("kfrc layout statistics {symbol}: {layout_stats:?}");
    }
    profile::measure("verify_l", &symbol, || {
        kfrc_son::verify::prepare_pipeline_output(&mut graph)
    })
    .map_err(|error| fail("layout verification", format!("{error}\n{}", graph.dump())))?;
    if std::env::var_os("KFRC_DUMP_GRAPH").is_some() {
        eprintln!("kfrc layout output {symbol}:\n{}", graph.dump());
    }
    let function =
        crate::machine::lower(&graph).map_err(|error| fail("machine lowering", error))?;
    Ok((function, graph.data_objects))
}

/// Every rustc-dependent answer is owned before entering the worker queue.
pub(crate) enum GraphInput {
    Body(kfrc_body::Body),
    Specialized {
        graph: kfrc_son::FunctionGraph,
        dirty_nodes: Vec<kfrc_son::NodeId>,
        local_slots: Vec<kfrc_son::NodeId>,
    },
}

pub(crate) struct Permissions {
    pub arguments: Vec<kfrc_son::ArgPermission>,
    pub calls: BTreeMap<String, Vec<kfrc_son::ArgPermission>>,
}

pub(crate) struct GraphJob {
    pub symbol: String,
    pub input: GraphInput,
    pub permissions: Option<Permissions>,
}

pub(crate) fn prepare_graph(job: GraphJob) -> Result<kfrc_son::FunctionGraph, CompileError> {
    let GraphJob {
        symbol,
        input,
        permissions,
    } = job;
    let fail = |stage: &str, error: String| CompileError {
        symbol: symbol.clone(),
        message: format!("{stage}: {error}"),
    };
    let (mut graph, specialization_seeds) = match input {
        GraphInput::Body(body) => {
            let graph = profile::measure("build", &symbol, || kfrc_son::build_graph(&body))
                .map_err(|error| fail("SoN construction", error.to_string()))?;
            (graph, None)
        }
        GraphInput::Specialized {
            graph,
            dirty_nodes,
            local_slots,
        } => (graph, Some((dirty_nodes, local_slots))),
    };
    if let Some(permissions) = permissions {
        graph.arg_permissions = permissions.arguments;
        graph.callee_permissions = permissions
            .calls
            .into_iter()
            .filter(|(symbol, _)| graph.callees.iter().any(|target| &target.symbol == symbol))
            .collect();
    }
    profile::measure("verify_s_input", &symbol, || {
        kfrc_son::verify_graph(&mut graph)
    })
    .map_err(|error| {
        fail(
            "semantic verification",
            format!("{error}\n{}", graph.dump()),
        )
    })?;
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
        .map_err(|error| fail("semantic optimization", error.to_string()))?;
        if std::env::var_os("KFRC_DUMP_GRAPH").is_some() {
            eprintln!("kfrc optimization {symbol}: {stats:?}");
        }
    }
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kfrc_son::GraphProvider;

    #[test]
    fn never_keeps_both_source_views_hidden_even_with_conflicting_strong_intent() {
        let mut graphs = CrateGraphs::default();
        graphs.bodies.insert(
            "blocked".into(),
            kfrc_son::FunctionGraph::new(
                "blocked",
                kfrc_body::FunctionAbi {
                    args: vec![],
                    ret: kfrc_body::PassMode::Ignore,
                    c_variadic: false,
                },
                kfrc_body::RecordedTypes::new(),
            ),
        );
        graphs.no_inline.insert("blocked".into());
        graphs.always_inline.insert("blocked".into());
        let provider = CrateProvider { all: &graphs };
        assert!(provider.always_inline("blocked"));
        assert!(provider.graph("blocked").is_none());
        assert!(provider.preparation_graph("blocked").is_none());
    }
}
