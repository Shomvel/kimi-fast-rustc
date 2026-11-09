//! Replace an If operand only when an earlier executed predicate proves it.
//! Proofs belong to one immutable CFG snapshot. Receipts are verified before
//! committing the batch; no condition value is replaced outside its If use.

use std::collections::{HashMap, HashSet};

use crate::analysis::cfg::ControlFlow;
use crate::analysis::dominators::Dominators;
use crate::analysis::refinement::{ConditionReceipt, PredicateFacts, canonical_condition};
use crate::verify::VerifyError;
use crate::{FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ConditionStats {
    pub conditions_folded: usize,
    pub fact_queries: usize,
    pub fact_node_visits: usize,
    pub receipts_checked: usize,
}

pub(crate) fn run(graph: &mut FunctionGraph) -> Result<ConditionStats, VerifyError> {
    let mut stats = ConditionStats::default();
    if graph.level != GraphLevel::Specialized {
        return Ok(stats);
    }
    // Reuse the witness scan to discover whether this graph has any target.
    // Without an If there is no rewrite, even when Assume facts are present.
    // Do not build a CFG or dominators just to establish that empty result.
    let mut has_branch = false;
    let mut protected = HashSet::new();
    for node in graph.ids() {
        stats.fact_node_visits += 1;
        match graph.op(node) {
            Opcode::If => has_branch = true,
            Opcode::VariantWitness => {
                // Removing a branch which supplies a witness would erase its
                // evidence. Resolve explicitly because edges are not normalized
                // until after the no-target check.
                let proof = graph.resolve(graph.input(node, 1));
                if graph.op(proof) == Opcode::Projection {
                    protected.insert(graph.resolve(graph.input(proof, 0)));
                }
            }
            _ => {}
        }
    }
    if !has_branch {
        return Ok(stats);
    }
    graph.resolve_edges();
    let cfg = ControlFlow::new(graph);
    let dominators = Dominators::new(&cfg);
    let mut evidence: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    let mut comparisons: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for &node in &cfg.order {
        if graph.op(node) != Opcode::Projection || graph.value_type(node) != &ValueType::Control {
            continue;
        }
        let source = graph.input(node, 0);
        let condition = match (graph.op(source), graph.payload(node)) {
            (Opcode::If, 0 | 1) => graph.input(source, 0),
            (Opcode::Assume, 0) => graph.input(source, 1),
            _ => continue,
        };
        if let Some((base, _)) = canonical_condition(graph, condition) {
            evidence.entry(base).or_default().push(node);
            if graph.op(base) == Opcode::Compare {
                for &operand in graph.inputs(base) {
                    comparisons
                        .entry(graph.resolve(operand))
                        .or_default()
                        .push(node);
                }
            }
        }
    }
    let mut facts = PredicateFacts::new(graph, &cfg, &dominators);
    let mut receipts = Vec::new();
    for &branch in &cfg.order {
        if graph.op(branch) != Opcode::If || protected.contains(&branch) {
            continue;
        }
        let condition = graph.resolve(graph.input(branch, 0));
        let Some((base, _)) = canonical_condition(graph, condition) else {
            continue;
        };
        if graph.op(base) == Opcode::Const {
            continue;
        }
        // Try exact predicates first, then comparisons sharing an SSA operand.
        // Dominance and dynamic-instance validation decide proof validity.
        let mut candidates: Vec<_> = evidence.get(&base).into_iter().flatten().copied().collect();
        if graph.op(base) == Opcode::Compare {
            for &operand in graph.inputs(base) {
                candidates.extend(
                    comparisons
                        .get(&graph.resolve(operand))
                        .into_iter()
                        .flatten()
                        .copied(),
                );
            }
        }
        let mut tried = HashSet::new();
        for proof in candidates {
            if !tried.insert(proof) {
                continue;
            }
            stats.fact_queries += 1;
            if let Some(truth) = facts.condition_at(condition, proof, graph.control(branch)) {
                receipts.push(ConditionReceipt {
                    branch,
                    condition,
                    proof,
                    truth,
                });
                break;
            }
        }
    }
    stats.fact_node_visits += facts.stats().node_visits + cfg.order.len();
    #[cfg(debug_assertions)]
    {
        crate::verify::condition_receipts(graph, &receipts)?;
        stats.receipts_checked = receipts.len();
    }
    for receipt in receipts {
        let ty = graph
            .rust_type(receipt.condition)
            .expect("Boolean condition");
        let constant = graph.add_constant(u128::from(receipt.truth), ty);
        graph.set_input(receipt.branch, 0, constant);
        stats.conditions_folded += 1;
    }
    Ok(stats)
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;
    use crate::{GraphLevel, verify_graph};
    use kfrc_body::{FunctionAbi, PassMode, RecordedTypes, TypeShape};

    #[test]
    fn receipt_verifier_rejects_false_sibling_and_stale_evidence() {
        let mut types = RecordedTypes::new();
        let boolean = types.intern(TypeShape::Bool, None);
        let mut graph = FunctionGraph::new(
            "receipt",
            FunctionAbi {
                args: vec![PassMode::Direct],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Specialized;
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(boolean),
            ]),
        );
        graph.start = start;
        let control = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            0,
            ValueType::Control,
        );
        let effect = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            1,
            ValueType::Effect,
        );
        let condition = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            2,
            ValueType::Rust(boolean),
        );
        let outer = graph.add_node(
            Opcode::If,
            control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        let yes = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[outer],
            0,
            ValueType::Control,
        );
        let no = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[outer],
            1,
            ValueType::Control,
        );
        graph.add_node(
            Opcode::Return,
            no,
            &[effect, condition],
            0,
            ValueType::Control,
        );
        let inner = graph.add_node(
            Opcode::If,
            yes,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        for slot in 0..2 {
            let edge = graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[inner],
                slot,
                ValueType::Control,
            );
            graph.add_node(
                Opcode::Return,
                edge,
                &[effect, condition],
                0,
                ValueType::Control,
            );
        }
        verify_graph(&mut graph).unwrap();
        let receipt = ConditionReceipt {
            branch: inner,
            condition,
            proof: yes,
            truth: true,
        };
        crate::verify::condition_receipts(&graph, &[receipt]).unwrap();
        assert!(
            crate::verify::condition_receipts(
                &graph,
                &[ConditionReceipt {
                    truth: false,
                    ..receipt
                }]
            )
            .is_err()
        );
        assert!(
            crate::verify::condition_receipts(
                &graph,
                &[ConditionReceipt {
                    proof: no,
                    ..receipt
                }]
            )
            .is_err()
        );
        let not = graph.add_node(
            Opcode::BitNot,
            NodeId::NONE,
            &[condition],
            0,
            ValueType::Rust(boolean),
        );
        graph.set_input(inner, 0, not);
        assert!(crate::verify::condition_receipts(&graph, &[receipt]).is_err());
    }
}
