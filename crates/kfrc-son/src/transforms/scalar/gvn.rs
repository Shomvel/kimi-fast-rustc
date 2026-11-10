//! Exact expression numbering for floating, deterministic value nodes.
//!
//! This deliberately does not number memory reads, projections, Phi, calls,
//! traps, addresses or control-anchored values. Equal pointer comparisons do
//! not establish interchangeable pointers; we only reuse the comparison's
//! result when its original operands are identical.
//! VariantField keys retain their optional witness input. Two equal subjects
//! under different proof scopes are not interchangeable expressions, and the
//! compile-time witnesses themselves never enter value numbering. A witness
//! whose proof point is dominated by an equal proof's point is that proof:
//! the subject is an immutable SSA value, so the dominating proof holds there.

use kfrc_body::{TypeOracle, TypeShape, fx_hash::HashMap};

use crate::analysis::cfg::ControlFlow;
use crate::analysis::dominators::Dominators;
use crate::graph::{FunctionGraph, NodeId, Opcode, ValueType};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GvnStats {
    pub values_merged: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Expression {
    opcode: Opcode,
    ty: ValueType,
    payload: u32,
    constant: Option<u128>,
    inputs: Vec<NodeId>,
}

pub(crate) fn run(graph: &mut FunctionGraph) -> GvnStats {
    let mut stats = GvnStats::default();
    stats.values_merged += merge_dominated_witnesses(graph);
    let mut expressions = HashMap::default();
    let nodes: Vec<_> = graph.ids().collect();
    for node in nodes {
        if !graph.control(node).is_none()
            || !matches!(graph.value_type(node), ValueType::Rust(_))
            || !numberable(graph.op(node))
        {
            continue;
        }
        let constant = (graph.op(node) == Opcode::Const).then(|| graph.constant_value(node));
        let mut inputs: Vec<_> = graph
            .inputs(node)
            .iter()
            .map(|&input| graph.resolve(input))
            .collect();
        // Compare commutative integer expressions without changing the operand
        // order used by instruction selection and tied-register allocation.
        if matches!(
            graph.op(node),
            Opcode::Add | Opcode::Mul | Opcode::BitAnd | Opcode::BitOr | Opcode::BitXor
        ) && graph.payload(node) == 0
            && graph.rust_type(node).is_some_and(|ty| {
                matches!(
                    graph.oracle.shape(ty),
                    TypeShape::Int { .. } | TypeShape::Bool
                )
            })
        {
            inputs.sort_unstable();
        }
        let expression = Expression {
            opcode: graph.op(node),
            ty: graph.value_type(node).clone(),
            payload: if constant.is_some() {
                0
            } else {
                graph.payload(node)
            },
            constant,
            inputs,
        };
        if let Some(&existing) = expressions.get(&expression) {
            graph.replace(node, existing);
            stats.values_merged += 1;
        } else {
            expressions.insert(expression, node);
        }
    }
    stats
}

/// Replaces each `VariantWitness` with an equal witness (same subject and
/// variant) whose proof control dominates its own.
fn merge_dominated_witnesses(graph: &mut FunctionGraph) -> usize {
    let mut groups: HashMap<(NodeId, u32), Vec<NodeId>> = HashMap::default();
    for node in graph.ids() {
        if graph.op(node) == Opcode::VariantWitness && !graph.is_dead(node) {
            let subject = graph.resolve(graph.input(node, 0));
            groups.entry((subject, graph.payload(node))).or_default().push(node);
        }
    }
    groups.retain(|_, witnesses| witnesses.len() > 1);
    if groups.is_empty() {
        return 0;
    }
    let cfg = ControlFlow::new(graph);
    let dominators = Dominators::new(&cfg);
    let mut merged = 0;
    let mut keys: Vec<_> = groups.keys().copied().collect();
    keys.sort_unstable();
    for key in keys {
        // Reverse postorder visits every dominator before the nodes it dominates.
        let mut witnesses: Vec<_> = groups[&key]
            .iter()
            .filter_map(|&w| {
                let control = graph.resolve(graph.control(w));
                cfg.position.get(&control).map(|&position| (position, w, control))
            })
            .collect();
        witnesses.sort_unstable();
        let mut kept: Vec<(NodeId, NodeId)> = Vec::new();
        for (_, witness, control) in witnesses {
            if let Some(&(proof, _)) = kept
                .iter()
                .find(|(_, kept_control)| dominators.dominates(&cfg, *kept_control, control))
            {
                graph.replace(witness, proof);
                merged += 1;
            } else {
                kept.push((witness, control));
            }
        }
    }
    merged
}

fn numberable(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Const
            | Opcode::Add
            | Opcode::Sub
            | Opcode::Mul
            | Opcode::BitAnd
            | Opcode::BitOr
            | Opcode::BitXor
            | Opcode::ShiftLeft
            | Opcode::ShiftRightArithmetic
            | Opcode::ShiftRightLogical
            | Opcode::Negate
            | Opcode::BitNot
            | Opcode::Compare
            | Opcode::IntCast
            | Opcode::MulOverflow
            | Opcode::BitCount
            | Opcode::ByteSwap
            | Opcode::BitReverse
            | Opcode::FunnelShift
            | Opcode::SaturatingAdd
            | Opcode::SaturatingSub
            | Opcode::Aggregate
            | Opcode::Repeat
            | Opcode::Field
            | Opcode::InsertField
            | Opcode::MakeVariant
            | Opcode::VariantOf
            | Opcode::VariantField
            | Opcode::Discriminant
            | Opcode::WidePtr
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GraphLevel, Interpreter, SemanticOutcome, Value, verify_graph};
    use kfrc_body::{
        FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SemanticAggregate, TypeShape,
    };

    #[test]
    fn equal_fields_keep_distinct_branch_witnesses() {
        let mut types = RecordedTypes::new();
        let boolean = types.intern(TypeShape::Bool, None);
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            None,
        );
        let choice = types.intern(
            TypeShape::Adt {
                name: "Choice".into(),
            },
            None,
        );
        types.record_semantic_aggregate(
            choice,
            SemanticAggregate {
                fields: vec![],
                variants: vec![vec![word]],
            },
        );
        types.record_semantic_enum(choice, vec![0]);
        let mut graph = FunctionGraph::new(
            "witness_scope",
            FunctionAbi {
                args: vec![PassMode::Direct, PassMode::Direct],
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
                ValueType::Rust(word),
            ]),
        );
        graph.start = start;
        let mut project =
            |index, ty| graph.add_node(Opcode::Projection, NodeId::NONE, &[start], index, ty);
        let control = project(0, ValueType::Control);
        let effect = project(1, ValueType::Effect);
        let condition = project(2, ValueType::Rust(boolean));
        let argument = project(3, ValueType::Rust(word));
        let value = graph.add_node(
            Opcode::MakeVariant,
            NodeId::NONE,
            &[argument],
            0,
            ValueType::Rust(choice),
        );
        let branch = graph.add_node(
            Opcode::If,
            control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        let mut fields = Vec::new();
        for side in 0..2 {
            let control = graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[branch],
                side,
                ValueType::Control,
            );
            let witness = graph.add_node(
                Opcode::VariantWitness,
                control,
                &[value, control],
                0,
                ValueType::Witness,
            );
            let payload = graph.intern_variant_field(0, 0);
            let field = graph.add_node(
                Opcode::VariantField,
                NodeId::NONE,
                &[value, witness],
                payload,
                ValueType::Rust(word),
            );
            fields.push(field);
            graph.add_node(
                Opcode::Return,
                control,
                &[effect, field],
                0,
                ValueType::Control,
            );
        }
        verify_graph(&mut graph).unwrap();
        assert_eq!(run(&mut graph).values_merged, 0);
        assert_ne!(graph.resolve(fields[0]), graph.resolve(fields[1]));
        verify_graph(&mut graph).unwrap();
        for side in [0, 1] {
            let result = Interpreter::new(&graph)
                .run_values(
                    &[Value::Scalar(side), Value::Scalar(42)],
                    &mut |_: &str, _: &[Value]| unreachable!(),
                )
                .unwrap();
            assert_eq!(result, SemanticOutcome::Return(Value::Scalar(42)));
        }
    }

    /// One enum proof at entry dominates both branches' proofs.
    #[test]
    fn a_dominating_witness_replaces_the_branch_witnesses_it_dominates() {
        let mut types = RecordedTypes::new();
        let boolean = types.intern(TypeShape::Bool, None);
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            None,
        );
        let choice = types.intern(
            TypeShape::Adt {
                name: "Choice".into(),
            },
            None,
        );
        types.record_semantic_aggregate(
            choice,
            SemanticAggregate {
                fields: vec![],
                variants: vec![vec![word]],
            },
        );
        types.record_semantic_enum(choice, vec![0]);
        let mut graph = FunctionGraph::new(
            "witness_dominance",
            FunctionAbi {
                args: vec![PassMode::Direct, PassMode::Direct],
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
                ValueType::Rust(word),
            ]),
        );
        graph.start = start;
        let mut project =
            |index, ty| graph.add_node(Opcode::Projection, NodeId::NONE, &[start], index, ty);
        let control = project(0, ValueType::Control);
        let effect = project(1, ValueType::Effect);
        let condition = project(2, ValueType::Rust(boolean));
        let argument = project(3, ValueType::Rust(word));
        let value = graph.add_node(
            Opcode::MakeVariant,
            NodeId::NONE,
            &[argument],
            0,
            ValueType::Rust(choice),
        );
        let entry_witness = graph.add_node(
            Opcode::VariantWitness,
            control,
            &[value, control],
            0,
            ValueType::Witness,
        );
        let branch = graph.add_node(
            Opcode::If,
            control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        let payload = graph.intern_variant_field(0, 0);
        let mut fields = Vec::new();
        for side in 0..2 {
            let control = graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[branch],
                side,
                ValueType::Control,
            );
            let witness = graph.add_node(
                Opcode::VariantWitness,
                control,
                &[value, control],
                0,
                ValueType::Witness,
            );
            let field = graph.add_node(
                Opcode::VariantField,
                NodeId::NONE,
                &[value, witness],
                payload,
                ValueType::Rust(word),
            );
            fields.push(field);
            graph.add_node(
                Opcode::Return,
                control,
                &[effect, field],
                0,
                ValueType::Control,
            );
        }
        // Keep the entry proof used, as a real producer would.
        let _ = entry_witness;
        verify_graph(&mut graph).unwrap();
        // Two witnesses and then their two equal fields merge.
        assert_eq!(run(&mut graph).values_merged, 3);
        assert_eq!(graph.resolve(fields[0]), graph.resolve(fields[1]));
        graph.resolve_edges();
        verify_graph(&mut graph).unwrap();
        for side in [0, 1] {
            let result = Interpreter::new(&graph)
                .run_values(
                    &[Value::Scalar(side), Value::Scalar(42)],
                    &mut |_: &str, _: &[Value]| unreachable!(),
                )
                .unwrap();
            assert_eq!(result, SemanticOutcome::Return(Value::Scalar(42)));
        }
    }
}
