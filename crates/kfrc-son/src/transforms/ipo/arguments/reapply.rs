//! Reapply existing scalar field facts to newly allocated preparation nodes.
//! No aggregate is reconstructed and no fact is inferred for a whole parameter.
use super::{Fact, MAX_FIELD_DEPTH};
use crate::{FunctionGraph, NodeId, Opcode};
use kfrc_body::{TypeOracle, TypeShape};

pub(super) struct Refinements {
    facts: Vec<Fact>,
    cursor: usize,
}

#[derive(Default)]
pub(super) struct Work {
    pub visits: usize,
    pub replaced: usize,
    pub allocated: usize,
    pub limited: bool,
}

impl Refinements {
    pub(super) fn new(mut facts: Vec<Fact>, cursor: usize) -> Self {
        // Whole scalar parameters were refined before any helper was visited.
        // This pass only needs facts about fields that can appear later.
        facts.retain(|fact| !fact.fields.is_empty());
        Self { facts, cursor }
    }

    pub(super) fn run(
        &mut self,
        graph: &mut FunctionGraph,
        visits: &mut usize,
        allocations: &mut usize,
    ) -> Work {
        let mut work = Work::default();
        if self.facts.is_empty() {
            self.cursor = graph.len();
            return work;
        }
        // Snapshot the end so this invocation never scans its own constants.
        let end = graph.len();
        while self.cursor < end {
            if !spend(visits, &mut work) {
                break;
            }
            let node = NodeId(self.cursor as u32);
            self.cursor += 1;
            if graph.is_dead(node)
                || graph.op(node) != Opcode::Field
                || !graph.control(node).is_none()
            {
                continue;
            }
            let Some((parameter, fields)) = parameter_field(graph, node, visits, &mut work) else {
                continue;
            };
            let Some(ty) = graph.rust_type(node) else {
                continue;
            };
            let shape = graph.oracle.shape(ty);
            if !matches!(
                shape,
                TypeShape::Int { .. } | TypeShape::Bool | TypeShape::Char
            ) {
                continue;
            }
            let Some(fact) = self.facts.iter().find(|fact| {
                fact.parameter == parameter && fact.fields == fields && &fact.shape == shape
            }) else {
                continue;
            };
            if *allocations == 0 {
                work.limited = true;
                continue;
            }
            // The original argument remains observed at the call boundary.
            // Like initial refinement, this fact holds when that value is
            // defined; replacing a field cannot excuse poison in another one.
            *allocations -= 1;
            let constant = graph.add_constant(fact.bits, ty);
            graph.replace(node, constant);
            work.replaced += 1;
            work.allocated += 1;
        }
        work
    }
}

fn spend(visits: &mut usize, work: &mut Work) -> bool {
    if *visits == 0 {
        work.limited = true;
        false
    } else {
        *visits -= 1;
        work.visits += 1;
        true
    }
}

fn parameter_field(
    graph: &FunctionGraph,
    mut node: NodeId,
    visits: &mut usize,
    work: &mut Work,
) -> Option<(u32, Vec<u32>)> {
    let mut fields = Vec::new();
    loop {
        if !spend(visits, work) {
            return None;
        }
        if !graph.control(node).is_none() {
            return None;
        }
        if graph.op(node) != Opcode::Field {
            break;
        }
        if fields.len() == MAX_FIELD_DEPTH {
            return None;
        }
        fields.push(graph.payload(node));
        node = graph.resolve(graph.input(node, 0));
    }
    if graph.op(node) != Opcode::Projection || graph.input(node, 0) != graph.start {
        return None;
    }
    let parameter = graph.payload(node).checked_sub(2)?;
    fields.reverse();
    Some((parameter, fields))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ValueType;
    use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TyId};

    fn fixture() -> (FunctionGraph, TyId, [NodeId; 2], Fact) {
        let mut types = RecordedTypes::new();
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let pair = types.intern(TypeShape::Tuple(vec![word, word]), None);
        let mut graph = FunctionGraph::new(
            "field_owner",
            FunctionAbi {
                args: vec![PassMode::Direct; 2],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(pair),
                ValueType::Rust(pair),
            ]),
        );
        let params = [2, 3].map(|slot| {
            graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[graph.start],
                slot,
                ValueType::Rust(pair),
            )
        });
        let fact = Fact {
            parameter: 0,
            fields: vec![1],
            shape: graph.oracle.shape(word).clone(),
            bits: 4,
        };
        (graph, word, params, fact)
    }

    #[test]
    fn different_root_parameter_field_path_or_scalar_type_cannot_reuse_a_fact() {
        for case in 0..4 {
            let (mut graph, word, params, mut fact) = fixture();
            if case == 2 {
                fact.shape = TypeShape::Bool;
            }
            let root = if case == 3 {
                let value = graph.add_constant(4, word);
                graph.add_node(
                    Opcode::Aggregate,
                    NodeId::NONE,
                    &[value, value],
                    0,
                    graph.value_type(params[0]).clone(),
                )
            } else {
                params[usize::from(case == 0)]
            };
            let mut refinements = Refinements::new(vec![fact], graph.len());
            let field = graph.add_node(
                Opcode::Field,
                NodeId::NONE,
                &[root],
                u32::from(case != 1),
                ValueType::Rust(word),
            );
            let mut visits = 100;
            let mut allocations = 2;
            let work = refinements.run(&mut graph, &mut visits, &mut allocations);
            assert_eq!(work.replaced, 0);
            assert_eq!(work.allocated, 0);
            assert_eq!(graph.resolve(field), field);
            assert_eq!(allocations, 2);
        }
    }

    #[test]
    fn repeated_triggers_and_partial_path_queries_share_nonrefundable_fuel() {
        for initial_visits in [0, 1, 7, usize::MAX] {
            for initial_allocations in [0, 2] {
                let (mut graph, word, params, fact) = fixture();
                let mut refinements = Refinements::new(vec![fact], graph.len());
                let mut visits = initial_visits;
                let mut allocations = initial_allocations;
                let mut charged_visits = 0;
                let mut charged_nodes = 0;
                for _ in 0..6 {
                    graph.add_node(
                        Opcode::Field,
                        NodeId::NONE,
                        &[params[0]],
                        1,
                        ValueType::Rust(word),
                    );
                    let before = graph.len();
                    let work = refinements.run(&mut graph, &mut visits, &mut allocations);
                    assert_eq!(graph.len() - before, work.allocated);
                    assert_eq!(work.replaced, work.allocated);
                    charged_visits += work.visits;
                    charged_nodes += work.allocated;
                }
                assert_eq!(charged_visits, initial_visits - visits);
                assert_eq!(charged_nodes, initial_allocations - allocations);
                assert!(charged_nodes <= 2);
                // Drain at most the last newly created constants, then old
                // Field nodes cannot allocate again on repeated cleanup.
                refinements.run(&mut graph, &mut visits, &mut allocations);
                let before = graph.len();
                for _ in 0..20 {
                    let work = refinements.run(&mut graph, &mut visits, &mut allocations);
                    assert_eq!(work.allocated, 0);
                    assert_eq!(work.replaced, 0);
                }
                assert_eq!(graph.len(), before);
            }
        }
    }
}
