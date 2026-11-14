//! Scoped integer equality propagation into fixed L Call operands. Facts are
//! checked at each original use; no SSA value is replaced globally.
use std::sync::OnceLock;

use kfrc_body::fx_hash::{HashMap, HashSet};

use crate::analysis::{
    cfg::ControlFlow,
    dominators::Dominators,
    refinement::{CallArgumentReceipt, PredicateFacts, integer_equality_on_edge, ordinary_integer},
};
use crate::verify::VerifyError;
use crate::{FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};

const MAX_FACT_QUERIES: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Stats {
    arguments_refined: usize,
    fact_queries: usize,
    node_visits: usize,
    receipts_checked: usize,
    budget_exhausted: bool,
}

fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("KFRC_LAYOUT_CALL_CONSTANTS").as_deref() != Ok("0"))
}

pub(super) fn run(graph: &mut FunctionGraph) -> Result<(), VerifyError> {
    if !enabled() {
        return Ok(());
    }
    let stats = refine(graph, MAX_FACT_QUERIES)?;
    static PRINT_STATS: OnceLock<bool> = OnceLock::new();
    if stats.fact_queries != 0
        && *PRINT_STATS
            .get_or_init(|| std::env::var("KFRC_LAYOUT_CALL_CONSTANTS_STATS").as_deref() == Ok("1"))
    {
        eprintln!(
            "kfrc-layout-call-constants: {} arguments_refined={} fact_queries={} node_visits={} receipts_checked={} budget_exhausted={}",
            graph.symbol,
            stats.arguments_refined,
            stats.fact_queries,
            stats.node_visits,
            stats.receipts_checked,
            stats.budget_exhausted,
        );
    }
    Ok(())
}

fn refine(graph: &mut FunctionGraph, query_budget: usize) -> Result<Stats, VerifyError> {
    let mut stats = Stats::default();
    if graph.level != GraphLevel::Layout {
        return Ok(stats);
    }
    let mut arguments = Vec::new();
    let mut values = HashSet::default();
    for call in graph.ids() {
        stats.node_visits += 1;
        if graph.op(call) != Opcode::Call {
            continue;
        }
        for (slot, &input) in graph.inputs(call).iter().enumerate().skip(1) {
            let input = graph.resolve(input);
            if graph.op(input) != Opcode::Const && ordinary_integer(graph, input).is_some() {
                arguments.push((call, slot, input));
                values.insert(input);
            }
        }
    }
    if arguments.is_empty() {
        return Ok(stats);
    }
    let mut evidence: HashMap<NodeId, Vec<NodeId>> = HashMap::default();
    for proof in graph.ids() {
        stats.node_visits += 1;
        if graph.op(proof) == Opcode::Projection
            && graph.value_type(proof) == &ValueType::Control
            && let Some((value, _)) = integer_equality_on_edge(graph, proof)
            && values.contains(&value)
        {
            evidence.entry(value).or_default().push(proof);
        }
    }
    // Do not construct CFG/dominators when no equality mentions a Call input.
    if evidence.is_empty() {
        return Ok(stats);
    }
    let cfg = ControlFlow::new(graph);
    let dominators = Dominators::new(&cfg);
    let mut facts = PredicateFacts::new(graph, &cfg, &dominators);
    let mut receipts = Vec::new();
    'arguments: for (call, slot, original) in arguments {
        for &proof in evidence.get(&original).into_iter().flatten() {
            if stats.fact_queries == query_budget {
                stats.budget_exhausted = true;
                break 'arguments;
            }
            stats.fact_queries += 1;
            if let Some(constant) =
                facts.integer_constant_at(original, proof, graph.resolve(graph.control(call)))
            {
                receipts.push(CallArgumentReceipt {
                    call,
                    slot,
                    original,
                    constant,
                    proof,
                });
                break;
            }
        }
    }
    stats.node_visits += facts.stats().node_visits;
    #[cfg(debug_assertions)]
    {
        crate::verify::call_argument_receipts(graph, &receipts)?;
        stats.receipts_checked = receipts.len();
    }
    for receipt in receipts {
        graph.set_input(receipt.call, receipt.slot, receipt.constant);
        stats.arguments_refined += 1;
    }
    Ok(stats)
}

#[cfg(test)]
#[path = "call_constants/tests.rs"]
mod tests;
