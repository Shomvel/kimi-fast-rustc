//! Share one reduced body across repeated literal calls. This runs after
//! bottom-up preparation, so profitable inlines have already disappeared.
use super::{CompileError, CrateGraphs};
use crate::emission::CguJob;
use kfrc_body::{TypeOracle, TypeShape};
use kfrc_object::SymbolBinding;
use kfrc_son::{CallTarget, FunctionGraph, NodeId, Opcode};
use std::collections::{BTreeMap, BTreeSet};

const MAX_SOURCE_NODES: usize = 1536;
const CRATE_NODE_BUDGET: usize = 8192;
const MIN_SHARED_CALLS: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Literal {
    parameter: usize,
    bits: u128,
}

struct Calls {
    shape: TypeShape,
    sites: Vec<(String, NodeId)>,
}

pub(crate) fn enabled() -> bool {
    std::env::var("KFRC_SHARED_LITERAL_CALLS").as_deref() == Ok("1")
        && std::env::var("KFRC_OPTIMIZE").as_deref() != Ok("0")
        && std::env::var("KFRC_INLINE").as_deref() != Ok("0")
}

impl CrateGraphs {
    pub(crate) fn specialize_literal_calls(
        &mut self,
        jobs: &mut [CguJob],
    ) -> Result<(), CompileError> {
        let mut owners = BTreeMap::new();
        let mut preemptible = BTreeSet::new();
        for (cgu, job) in jobs.iter().enumerate() {
            for (symbol, binding) in &job.pending_functions {
                owners.entry(symbol.clone()).or_insert(cgu);
                if matches!(binding, SymbolBinding::Global { hidden: false }) {
                    preemptible.insert(symbol.clone());
                }
            }
        }

        let mut groups = BTreeMap::<String, BTreeMap<Literal, Calls>>::new();
        for (caller, graph) in &self.bodies {
            for call in graph.ids().filter(|&node| graph.op(node) == Opcode::Call) {
                let index = graph.payload(call);
                let target = &graph.callees[index as usize];
                let symbol = &target.symbol;
                if caller == symbol
                    || !owners.contains_key(symbol)
                    || preemptible.contains(symbol)
                    || self.no_inline.contains(symbol)
                    || graph.runtime_comparison(index).is_some()
                {
                    continue;
                }
                let Some(source) = self.bodies.get(symbol) else {
                    continue;
                };
                if source.abi != target.abi || source.len() > MAX_SOURCE_NODES {
                    continue;
                }
                for (parameter, argument) in graph.inputs(call)[1..].iter().enumerate() {
                    let argument = graph.resolve(*argument);
                    if graph.op(argument) != Opcode::Const {
                        continue;
                    }
                    let Some(ty) = graph.rust_type(argument) else {
                        continue;
                    };
                    let shape = graph.oracle.shape(ty);
                    if !matches!(
                        shape,
                        TypeShape::Char | TypeShape::Bool | TypeShape::Int { .. }
                    ) {
                        continue;
                    }
                    let literal = Literal {
                        parameter,
                        bits: graph.constant_value(argument),
                    };
                    let group = groups
                        .entry(symbol.clone())
                        .or_default()
                        .entry(literal)
                        .or_insert_with(|| Calls {
                            shape: shape.clone(),
                            sites: Vec::new(),
                        });
                    if group.shape == *shape {
                        group.sites.push((caller.clone(), call));
                    }
                }
            }
        }

        let report = std::env::var_os("KFRC_DUMP_GRAPH").is_some()
            || std::env::var_os("KFRC_SPECIALIZATION_STATS").is_some();
        let mut budget = CRATE_NODE_BUDGET;
        let mut published = false;
        for (symbol, literals) in groups {
            // One variant per source keeps the cost independent of how many
            // different constants the caller population happens to contain.
            let Some((literal, group)) = literals
                .into_iter()
                .filter(|(_, group)| group.sites.len() >= MIN_SHARED_CALLS)
                .max_by_key(|(literal, group)| (group.sites.len(), std::cmp::Reverse(*literal)))
            else {
                continue;
            };
            let source = &self.bodies[&symbol];
            if source.len() > budget {
                continue;
            }
            let name = format!(
                "{symbol}.kfrc.literal.{}.{:x}",
                literal.parameter, literal.bits
            );
            if owners.contains_key(&name)
                || self.data.objects.contains_key(&name)
                || self.address_taken.contains(&name)
                || self.call_sites.contains_key(&name)
            {
                continue;
            }
            budget -= source.len();
            let Some(mut candidate) = kfrc_son::specialize_literal_argument(
                source,
                literal.parameter,
                &group.shape,
                literal.bits,
            )
            .map_err(|err| CompileError {
                symbol: symbol.clone(),
                message: format!("literal specialization: {err}"),
            })?
            else {
                continue;
            };
            let old_branches = branches(source);
            let new_branches = branches(&candidate);
            if old_branches.saturating_sub(new_branches) < 2
                || candidate.live_count() >= source.live_count()
            {
                continue;
            }
            candidate.symbol.clone_from(&name);
            let target = CallTarget {
                symbol: name.clone(),
                abi: candidate.abi.clone(),
            };
            for (caller, call) in &group.sites {
                let graph = self.bodies.get_mut(caller).expect("collected caller");
                let new = graph.add_callee(target.clone());
                if let Some(permissions) = graph.callee_permissions.get(&symbol) {
                    graph
                        .callee_permissions
                        .insert(name.clone(), permissions.clone());
                }
                // Keep the existing operands, control, and effects. In particular,
                // other arguments don't stop being observed when a clone ignores one.
                graph.set_payload(*call, new);
            }
            jobs[owners[&symbol]]
                .pending_functions
                .insert(name.clone(), SymbolBinding::Local);
            self.internal.insert(name.clone());
            self.inlining_complete.insert(name.clone());
            self.no_inline.insert(name.clone());
            self.bodies.insert(name.clone(), candidate);
            published = true;
            if report {
                eprintln!(
                    "kfrc literal clone {name}: sites={} branches={old_branches}->{new_branches}",
                    group.sites.len()
                );
            }
        }
        if published {
            self.collect_references();
        }
        Ok(())
    }
}

fn branches(graph: &FunctionGraph) -> usize {
    graph
        .ids()
        .filter(|&node| matches!(graph.op(node), Opcode::If | Opcode::VariantSwitch))
        .count()
}

#[cfg(test)]
mod tests;
