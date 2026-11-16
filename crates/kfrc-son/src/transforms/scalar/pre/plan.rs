use super::{PreStats, expression::Expression, proof::Proof};
use crate::analysis::uses::Uses;
use crate::{FunctionGraph, NodeId, Opcode, ValueType};
use kfrc_body::fx_hash::HashMap;

pub(super) struct Plan {
    source: NodeId,
    region: NodeId,
    op: Opcode,
    payload: u32,
    ty: ValueType,
    operands: [Vec<NodeId>; 2],
    existing: [Option<NodeId>; 2],
}

impl Plan {
    pub fn new(
        graph: &FunctionGraph,
        source: NodeId,
        expressions: &HashMap<Expression, Vec<NodeId>>,
        proof: &mut Proof<'_>,
    ) -> Option<Self> {
        proof.work.charge(1)?;
        let mut region = None;
        for &operand in graph.inputs(source) {
            let operand = graph.resolve(operand);
            if graph.op(operand) == Opcode::Phi {
                let origin = graph.input(operand, 0);
                if region.is_some_and(|region| region != origin) {
                    return None;
                }
                region = Some(origin);
            }
        }
        let region = region?;
        if graph.inputs(region).len() != 2 || !proof.acyclic(region) {
            return None;
        }
        let mut operands = [Vec::new(), Vec::new()];
        let mut existing = [None, None];
        for side in 0..2 {
            let pred = graph.input(region, side);
            if !proof.acyclic(pred) {
                return None;
            }
            for &operand in graph.inputs(source) {
                let operand = graph.resolve(operand);
                let value = if graph.op(operand) == Opcode::Phi {
                    if graph.inputs(operand).len() != 3 || graph.input(operand, 0) != region {
                        return None;
                    }
                    graph.resolve(graph.input(operand, side + 1))
                } else {
                    operand
                };
                if !proof.available(value, pred) {
                    return None;
                }
                operands[side].push(value);
            }
            let key = Expression::of(graph, source, &operands[side]);
            for &value in expressions.get(&key).into_iter().flatten() {
                proof.work.charge(1)?;
                if value != source && proof.needed_before(value, pred)? {
                    existing[side] = Some(value);
                    break;
                }
            }
        }
        if existing.iter().all(Option::is_none) {
            return None;
        }
        Some(Self {
            source,
            region,
            op: graph.op(source),
            payload: graph.payload(source),
            ty: graph.value_type(source).clone(),
            operands,
            existing,
        })
    }

    pub fn cost(&self) -> usize {
        1 + self.existing.iter().filter(|value| value.is_none()).count()
    }

    pub fn apply(self, graph: &mut FunctionGraph, uses: &Uses, stats: &mut PreStats) {
        let before = graph.len();
        let mut incoming = vec![self.region];
        for side in 0..2 {
            let value = match self.existing[side] {
                Some(value) => graph.resolve(value),
                None => {
                    stats.expressions_inserted += 1;
                    graph.add_node(
                        self.op,
                        NodeId::NONE,
                        &self.operands[side],
                        self.payload,
                        self.ty.clone(),
                    )
                }
            };
            incoming.push(value);
        }
        let phi = graph.add_node(Opcode::Phi, NodeId::NONE, &incoming, 0, self.ty);
        graph.replace(self.source, phi);
        stats.expressions_replaced += 1;
        stats.phis_created += 1;
        stats
            .dirty_nodes
            .extend(uses.of(self.source).iter().map(|usage| usage.user));
        stats
            .dirty_nodes
            .extend((before..graph.len()).map(|i| NodeId(i as u32)));
    }
}
