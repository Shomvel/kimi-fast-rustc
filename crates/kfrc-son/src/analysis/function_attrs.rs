//! Same-crate effects inferred from exact S bodies, independently of whether
//! execution returns. A no-write proof never licenses removing or moving a call.
//!
//! Memory and captures use a monotone fixed point: recursive calls start with
//! no effects, then every body access and escape propagates to its callers.
//! `willreturn` starts false instead; cycles cannot prove their own termination.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use kfrc_body::FunctionAbi;

use crate::analysis::cfg::ControlFlow;
use crate::analysis::memory::MemoryFacts;
use crate::analysis::uses::Uses;
use crate::{FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};

mod arguments;
mod memory;

/// Effects on memory visible to the caller. `Unknown` includes writes, frees,
/// synchronization and operations whose memory contract is not represented.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemoryEffects {
    None,
    ReadOnly,
    Unknown,
}

impl MemoryEffects {
    pub fn cannot_write(self) -> bool {
        self != Self::Unknown
    }
}

/// Proven uses of a pointer parameter. Capture includes returning the pointer,
/// storing it as a value, exposing its bits, or passing it to unknown code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArgumentAttrs {
    pub nocapture: bool,
    pub memory: MemoryEffects,
}

impl ArgumentAttrs {
    const UNKNOWN: Self = Self {
        nocapture: false,
        memory: MemoryEffects::Unknown,
    };

    const UNUSED: Self = Self {
        nocapture: true,
        memory: MemoryEffects::None,
    };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionAttrs {
    /// The source ABI guards against consuming proofs after a signature edit.
    pub abi: FunctionAbi,
    pub memory: MemoryEffects,
    /// S parameter order, including parameters physically passed as Ignore.
    pub arguments: Vec<ArgumentAttrs>,
    pub noreturn: bool,
    pub willreturn: bool,
}

impl FunctionAttrs {
    fn unknown(graph: &FunctionGraph) -> Self {
        Self {
            abi: graph.abi.clone(),
            memory: MemoryEffects::Unknown,
            arguments: vec![ArgumentAttrs::UNKNOWN; parameter_types(graph).len()],
            noreturn: false,
            willreturn: false,
        }
    }
}

struct ArgumentFacts {
    attrs: ArgumentAttrs,
    calls: Vec<(String, usize)>,
}

struct BodyFacts {
    attrs: FunctionAttrs,
    arguments: Vec<ArgumentFacts>,
    calls: Vec<String>,
    returning_cfg: bool,
}

/// Infer from definitions the caller has already proved exact. For a crate
/// table that can include replaceable definitions, use `infer_exact` instead.
pub fn infer(bodies: &BTreeMap<String, FunctionGraph>) -> BTreeMap<String, FunctionAttrs> {
    infer_exact(bodies, &bodies.keys().cloned().collect())
}

/// Missing or replaceable definitions, mismatched ABIs, indirect calls and
/// barriers have unknown effects. Existing metadata is deliberately not an
/// input, so reinference cannot retain a stale signature/body assumption.
pub fn infer_exact(
    bodies: &BTreeMap<String, FunctionGraph>,
    exact: &BTreeSet<String>,
) -> BTreeMap<String, FunctionAttrs> {
    let facts: BTreeMap<_, _> = bodies
        .iter()
        .map(|(symbol, graph)| (symbol.clone(), body_facts(symbol, graph, bodies, exact)))
        .collect();
    let mut attrs: BTreeMap<_, _> = facts
        .iter()
        .map(|(symbol, facts)| (symbol.clone(), facts.attrs.clone()))
        .collect();
    let mut callers: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (symbol, facts) in &facts {
        for callee in &facts.calls {
            callers.entry(callee).or_default().insert(symbol);
        }
    }
    let mut queue: VecDeque<_> = facts.keys().map(String::as_str).collect();
    let mut queued: BTreeSet<_> = facts.keys().map(String::as_str).collect();
    while let Some(symbol) = queue.pop_front() {
        queued.remove(symbol);
        let body = &facts[symbol];
        let mut next = body.attrs.clone();
        next.willreturn = body.returning_cfg;
        for callee in &body.calls {
            next.memory = next.memory.max(attrs[callee].memory);
            next.willreturn &= attrs[callee].willreturn;
        }
        for (argument, facts) in next.arguments.iter_mut().zip(&body.arguments) {
            *argument = facts.attrs;
            for (callee, index) in &facts.calls {
                let callee = attrs[callee]
                    .arguments
                    .get(*index)
                    .copied()
                    .unwrap_or(ArgumentAttrs::UNKNOWN);
                argument.nocapture &= callee.nocapture;
                argument.memory = argument.memory.max(callee.memory);
            }
            // A captured copy may be reloaded and written through later. Do
            // not assign an access attribute from the visible uses alone.
            if !argument.nocapture {
                argument.memory = MemoryEffects::Unknown;
            }
        }
        if next == attrs[symbol] {
            continue;
        }
        attrs.insert(symbol.to_owned(), next);
        for &caller in callers.get(symbol).into_iter().flatten() {
            if queued.insert(caller) {
                queue.push_back(caller);
            }
        }
    }
    attrs
}

fn body_facts(
    symbol: &str,
    graph: &FunctionGraph,
    bodies: &BTreeMap<String, FunctionGraph>,
    exact: &BTreeSet<String>,
) -> BodyFacts {
    let mut result = BodyFacts {
        attrs: FunctionAttrs::unknown(graph),
        arguments: Vec::new(),
        calls: Vec::new(),
        returning_cfg: false,
    };
    if graph.level != GraphLevel::Specialized
        || graph.symbol != symbol
        || !exact.contains(symbol)
        || graph.start.index() >= graph.len()
        || graph.abi.c_variadic
        || parameter_types(graph).len() != graph.abi.args.len()
    {
        return result;
    }
    let cfg = ControlFlow::new(graph);
    result.attrs.memory = MemoryEffects::None;
    result.attrs.noreturn = !cfg.order.iter().any(|&n| graph.op(n) == Opcode::Return);
    result.returning_cfg = returns_without_loop(graph, &cfg);
    let memory = MemoryFacts::new(graph);
    for node in graph.ids() {
        if graph.op(node) == Opcode::Call {
            if let Some(callee) = direct_callee(graph, node, bodies) {
                result.calls.push(callee.to_owned());
            } else {
                result.attrs.memory = MemoryEffects::Unknown;
                result.returning_cfg = false;
            }
        } else {
            result.attrs.memory = result
                .attrs
                .memory
                .max(memory::local_effect(graph, node, &memory));
            if matches!(
                graph.op(node),
                Opcode::CallIndirect
                    | Opcode::TraitCall
                    | Opcode::Opaque
                    | Opcode::ThreadLocalAddress
            ) {
                result.returning_cfg = false;
            }
        }
    }
    result.calls.sort();
    result.calls.dedup();
    let uses = Uses::new(graph);
    result.arguments = parameter_types(graph)
        .iter()
        .enumerate()
        .map(|(index, ty)| arguments::argument_facts(graph, index, ty, &uses, bodies))
        .collect();
    result.attrs.arguments = result.arguments.iter().map(|a| a.attrs).collect();
    result
}

fn parameter_types(graph: &FunctionGraph) -> &[ValueType] {
    if graph.start.index() >= graph.len() {
        return &[];
    }
    match graph.value_type(graph.start) {
        ValueType::Tuple(types) => types.get(2..).unwrap_or(&[]),
        _ => &[],
    }
}

fn direct_callee<'a>(
    graph: &'a FunctionGraph,
    call: NodeId,
    bodies: &BTreeMap<String, FunctionGraph>,
) -> Option<&'a str> {
    let target = graph.callees.get(graph.payload(call) as usize)?;
    let callee = bodies.get(&target.symbol)?;
    (target.abi == callee.abi && graph.inputs(call).len() == parameter_types(callee).len() + 1)
        .then_some(target.symbol.as_str())
}

fn returns_without_loop(graph: &FunctionGraph, cfg: &ControlFlow) -> bool {
    if cfg.order.is_empty() {
        return false;
    }
    let mut remaining: Vec<_> = cfg.predecessors.iter().map(Vec::len).collect();
    let mut queue: VecDeque<_> = remaining
        .iter()
        .enumerate()
        .filter_map(|(index, &count)| (count == 0).then_some(index))
        .collect();
    let mut visited = 0;
    while let Some(index) = queue.pop_front() {
        visited += 1;
        let node = cfg.order[index];
        if cfg.successors(node).is_empty() && graph.op(node) != Opcode::Return {
            return false;
        }
        for successor in cfg.successors(node) {
            let index = cfg.position[successor];
            remaining[index] -= 1;
            if remaining[index] == 0 {
                queue.push_back(index);
            }
        }
    }
    visited == cfg.order.len()
}

#[cfg(test)]
mod tests;
