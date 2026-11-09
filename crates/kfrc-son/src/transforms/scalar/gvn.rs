//! Exact expression numbering for floating, deterministic value nodes.
//!
//! This deliberately does not number memory reads, projections, Phi, calls,
//! traps, addresses or control-anchored values. Equal pointer comparisons do
//! not establish interchangeable pointers; we only reuse the comparison's
//! result when its original operands are identical.
//! VariantField keys retain their optional witness input. Two equal subjects
//! under different proof scopes are not interchangeable expressions, and the
//! compile-time witnesses themselves never enter value numbering.

use std::collections::HashMap;

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
    let mut expressions = HashMap::new();
    let nodes: Vec<_> = graph.ids().collect();
    for node in nodes {
        if !graph.control(node).is_none()
            || !matches!(graph.value_type(node), ValueType::Rust(_))
            || !numberable(graph.op(node))
        {
            continue;
        }
        let constant = (graph.op(node) == Opcode::Const).then(|| graph.constant_value(node));
        let expression = Expression {
            opcode: graph.op(node),
            ty: graph.value_type(node).clone(),
            payload: if constant.is_some() {
                0
            } else {
                graph.payload(node)
            },
            constant,
            inputs: graph
                .inputs(node)
                .iter()
                .map(|&input| graph.resolve(input))
                .collect(),
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
}
