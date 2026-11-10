//! Owned graph compilation. No rustc context crosses the worker boundary.
use crate::{inline_cost, profile};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct CompileError {
    pub symbol: String,
    pub message: String,
}

#[derive(Default)]
pub(crate) struct CrateGraphs {
    pub(crate) bodies: BTreeMap<String, kfrc_son::FunctionGraph>,
    pub(crate) no_inline: BTreeSet<String>,
    pub(crate) internal: BTreeSet<String>,
    pub(crate) references: BTreeMap<String, BTreeSet<String>>,
    /// Direct call sites per callee, by calling function.
    pub(crate) call_sites: BTreeMap<String, BTreeMap<String, usize>>,
    /// Symbols referenced other than by a direct call.
    pub(crate) address_taken: BTreeSet<String>,
}

impl CrateGraphs {
    pub(crate) fn collect_references(&mut self) {
        self.references.clear();
        self.call_sites.clear();
        self.address_taken.clear();
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

    /// Prepare small wrappers before they are priced at their callers. Each
    /// wave reads immutable prior-wave bodies; recursive inlining is rejected
    /// by the normal inliner. Keep shrinking results or small bodies with fewer calls; cap total work.
    pub(crate) fn simplify_wrappers(&mut self) -> Result<(), CompileError> {
        if std::env::var("KFRC_OPTIMIZE").as_deref() == Ok("0")
            || std::env::var("KFRC_INLINE").as_deref() == Ok("0")
        {
            return Ok(());
        }
        let available = BTreeSet::new();
        let mut fuel = 100_000usize;
        for _ in 0..2 {
            let provider = CguGraphs {
                all: self,
                available: &available,
            };
            let mut updates = Vec::new();
            for (symbol, source) in &self.bodies {
                let size = source.live_count();
                if size > 128 || !source.ids().any(|n| source.op(n) == kfrc_son::Opcode::Call) {
                    continue;
                }
                if fuel < size {
                    break;
                }
                fuel -= size;
                let fail = |error: String| CompileError {
                    symbol: symbol.clone(),
                    message: format!("wrapper preparation: {error}"),
                };
                let mut candidate = source.clone();
                let identities = kfrc_son::fold_identity_calls(&mut candidate, &provider, None)
                    .map_err(|e| fail(e.to_string()))?;
                let inlined = kfrc_son::inline_calls(
                    &mut candidate,
                    &provider,
                    kfrc_son::InlineBudget {
                        max_depth: 2,
                        max_nodes: 256,
                        max_calls: 4,
                        max_callee_nodes: 64,
                        copy_anonymous_constants: true,
                        ..kfrc_son::InlineBudget::default()
                    },
                )
                .map_err(|e| fail(e.to_string()))?;
                if identities.calls_folded + inlined.calls_inlined == 0 {
                    continue;
                }
                let seeds: Vec<_> = identities
                    .dirty_nodes
                    .into_iter()
                    .chain(inlined.dirty_nodes)
                    .collect();
                let slots: Vec<_> = identities
                    .argument_slots
                    .into_iter()
                    .chain(inlined.argument_slots)
                    .collect();
                kfrc_son::optimize_specialized_seams(&mut candidate, &seeds, &slots)
                    .map_err(|e| fail(e.to_string()))?;
                let calls = |g: &kfrc_son::FunctionGraph| {
                    g.ids()
                        .filter(|&n| g.op(n) == kfrc_son::Opcode::Call)
                        .count()
                };
                if candidate.live_count() < size
                    || (candidate.live_count() <= 64 && calls(&candidate) < calls(source))
                {
                    updates.push((symbol.clone(), candidate));
                }
            }
            if updates.is_empty() {
                break;
            }
            self.bodies.extend(updates);
            self.collect_references();
        }
        Ok(())
    }
}

/// Inlining must not create relocations to another object's local functions.
/// Pure bodies remain available across CGUs; private symbol dependencies do not.
pub(crate) struct CguGraphs<'a> {
    pub(crate) all: &'a CrateGraphs,
    pub(crate) available: &'a BTreeSet<String>,
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

pub(crate) fn compile_function(
    mut graph: kfrc_son::FunctionGraph,
    provider: &CguGraphs<'_>,
) -> Result<(kfrc_x86::MachineFunction, Vec<kfrc_body::DataObject>), CompileError> {
    let symbol = graph.symbol.clone();
    let fail = |stage: &str, error: String| CompileError {
        symbol: symbol.clone(),
        message: format!("{stage}: {error}"),
    };
    if std::env::var("KFRC_OPTIMIZE").as_deref() != Ok("0") {
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
            let cost_policy =
                inline_cost::policy().map_err(|error| fail("inline cost configuration", error))?;
            let inlined = profile::measure("inline_s", &symbol, || {
                kfrc_son::inline_calls_with_cost(
                    &mut graph,
                    provider,
                    kfrc_son::InlineBudget::default(),
                    cost_policy,
                    &kfrc_son::analysis::cost::GenericCostModel,
                )
            })
            .map_err(|error| fail("semantic inlining", error.to_string()))?;
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
                    .map_err(|error| fail("nested identity simplification", error.to_string()))?;
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
    profile::measure("verify_s_output", &symbol, || {
        kfrc_son::verify_graph(&mut graph)
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
    let layout_stats = profile::measure("layout", &symbol, || kfrc_son::lower_layout(&mut graph))
        .map_err(|error| fail("layout lowering", error.to_string()))?;
    if std::env::var_os("KFRC_DUMP_GRAPH").is_some()
        || std::env::var_os("KFRC_SPECIALIZATION_STATS").is_some()
    {
        eprintln!("kfrc layout statistics {symbol}: {layout_stats:?}");
    }
    profile::measure("verify_l", &symbol, || kfrc_son::verify_graph(&mut graph))
        .map_err(|error| fail("layout verification", format!("{error}\n{}", graph.dump())))?;
    if std::env::var_os("KFRC_DUMP_GRAPH").is_some() {
        eprintln!("kfrc layout output {symbol}:\n{}", graph.dump());
    }
    let function = profile::measure("x86", &symbol, || kfrc_x86::lower_function(&graph))
        .map_err(|error| fail("x86 lowering", error.to_string()))?;
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
