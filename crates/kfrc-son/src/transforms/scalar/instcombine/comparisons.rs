//! Narrow comparisons through ordinary integer extensions. The replacement
//! still consumes the original source, including its poison and contracts.
use super::{Integer, constant};
use crate::{ComparePredicate, FunctionGraph, NodeId, Opcode};
use kfrc_body::{TyId, TypeOracle, TypeShape};

fn ordinary_integer(graph: &FunctionGraph, ty: TyId) -> Option<Integer> {
    if graph.oracle.scalar_validity(ty).is_some() {
        return None;
    }
    match graph.oracle.shape(ty) {
        TypeShape::Int { width, signed } if width.bits(64) <= 64 => Some(Integer {
            bits: width.bits(64),
            signed: *signed,
        }),
        _ => None,
    }
}

fn swapped(predicate: ComparePredicate) -> ComparePredicate {
    use ComparePredicate::*;
    match predicate {
        Equal | NotEqual => predicate,
        SignedLess => SignedGreater,
        SignedLessEqual => SignedGreaterEqual,
        SignedGreater => SignedLess,
        SignedGreaterEqual => SignedLessEqual,
        UnsignedLess => UnsignedGreater,
        UnsignedLessEqual => UnsignedGreaterEqual,
        UnsignedGreater => UnsignedLess,
        UnsignedGreaterEqual => UnsignedLessEqual,
    }
}

fn through_extension(predicate: ComparePredicate, signed: bool) -> ComparePredicate {
    use ComparePredicate::*;
    if signed {
        return predicate;
    }
    match predicate {
        SignedLess => UnsignedLess,
        SignedLessEqual => UnsignedLessEqual,
        SignedGreater => UnsignedGreater,
        SignedGreaterEqual => UnsignedGreaterEqual,
        _ => predicate,
    }
}

fn is_variant_evidence(graph: &FunctionGraph, comparison: NodeId) -> bool {
    graph.ids().any(|node| {
        if graph.op(node) != Opcode::VariantWitness {
            return false;
        }
        let proof = graph.resolve(graph.input(node, 1));
        if graph.op(proof) != Opcode::Projection {
            return false;
        }
        let source = graph.resolve(graph.input(proof, 0));
        let condition = match graph.op(source) {
            Opcode::If => graph.input(source, 0),
            Opcode::Assume => graph.input(source, 1),
            _ => return false,
        };
        crate::analysis::refinement::canonical_condition(graph, condition)
            .is_some_and(|(base, _)| base == comparison)
    })
}

/// IC-CMP-EXT-CONSTANT: compare an injective extension against a constant
/// that round-trips exactly. Out-of-range constants deliberately stay intact:
/// folding their comparison to a constant could erase source poison.
pub(super) fn combine(graph: &mut FunctionGraph, node: NodeId) -> Option<NodeId> {
    if graph.op(node) != Opcode::Compare || !graph.control(node).is_none() {
        return None;
    }
    let mut extension = graph.resolve(graph.input(node, 0));
    let mut constant_node = graph.resolve(graph.input(node, 1));
    let mut predicate = ComparePredicate::from_payload(graph.payload(node));
    if graph.op(extension) == Opcode::Const && graph.op(constant_node) == Opcode::IntCast {
        std::mem::swap(&mut extension, &mut constant_node);
        predicate = swapped(predicate);
    }
    if graph.op(extension) != Opcode::IntCast
        || graph.op(constant_node) != Opcode::Const
        || graph.payload(extension) != 0
        || !graph.control(extension).is_none()
    {
        return None;
    }
    let target_ty = graph.rust_type(extension)?;
    if graph.rust_type(constant_node) != Some(target_ty) {
        return None;
    }
    let target = ordinary_integer(graph, target_ty)?;
    let source_node = graph.resolve(graph.input(extension, 0));
    let source_ty = graph.rust_type(source_node)?;
    let source = ordinary_integer(graph, source_ty)?;
    if source.bits >= target.bits {
        return None;
    }
    let wide = constant(graph, constant_node)?;
    let narrow = source.normalize(wide);
    let extended = if source.signed {
        source.signed(narrow) as u64
    } else {
        narrow
    };
    if target.normalize(extended) != wide || is_variant_evidence(graph, node) {
        return None;
    }
    let narrow_constant = graph.add_constant(u128::from(narrow), source_ty);
    Some(graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[source_node, narrow_constant],
        through_extension(predicate, source.signed) as u32,
        graph.value_type(node).clone(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GraphLevel, ValueType, verify_graph};
    use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SemanticAggregate};

    #[test]
    fn unsupported_widths_and_invalid_cast_payloads_are_rejected_by_the_matcher() {
        // These are deliberately not valid Specialized graphs. W128 values
        // are outside that stage's subset, and IntCast has no legal flags.
        // Exercise only the defensive matcher boundary, never the pipeline.
        for (source_width, target_width, payload) in [
            (IntegerWidth::W8, IntegerWidth::W128, 0),
            (IntegerWidth::W128, IntegerWidth::W64, 0),
            (IntegerWidth::W8, IntegerWidth::W64, 1),
        ] {
            let mut types = RecordedTypes::new();
            let mut integer = |width| {
                types.intern(
                    TypeShape::Int {
                        width,
                        signed: false,
                    },
                    None,
                )
            };
            let source_ty = integer(source_width);
            let target_ty = integer(target_width);
            let boolean = types.intern(TypeShape::Bool, None);
            let mut graph = FunctionGraph::new(
                "matcher_rejection",
                FunctionAbi {
                    args: vec![],
                    ret: PassMode::Direct,
                    c_variadic: false,
                },
                types,
            );
            graph.level = GraphLevel::Specialized;
            let source = graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(source_ty),
            );
            let extension = graph.add_node(
                Opcode::IntCast,
                NodeId::NONE,
                &[source],
                payload,
                ValueType::Rust(target_ty),
            );
            let constant = graph.add_constant(17, target_ty);
            let comparison = graph.add_node(
                Opcode::Compare,
                NodeId::NONE,
                &[extension, constant],
                ComparePredicate::Equal as u32,
                ValueType::Rust(boolean),
            );
            let nodes = graph.len();
            assert!(combine(&mut graph, comparison).is_none());
            assert_eq!(graph.len(), nodes);
        }
    }

    #[test]
    fn variant_proof_conditions_are_not_rewritten_through_boolean_wrappers() {
        for assumption in [false, true] {
            for wrapper in 0..3 {
                let mut types = RecordedTypes::new();
                let byte = types.intern(
                    TypeShape::Int {
                        width: IntegerWidth::W8,
                        signed: false,
                    },
                    None,
                );
                let word = types.intern(
                    TypeShape::Int {
                        width: IntegerWidth::W64,
                        signed: false,
                    },
                    None,
                );
                let boolean = types.intern(TypeShape::Bool, None);
                let enumeration = types.intern(
                    TypeShape::Adt {
                        name: "KnownVariant".into(),
                    },
                    None,
                );
                types.record_semantic_aggregate(
                    enumeration,
                    SemanticAggregate {
                        fields: vec![],
                        variants: vec![vec![byte]],
                    },
                );
                types.record_semantic_enum(enumeration, vec![0]);
                let mut graph = FunctionGraph::new(
                    "protected_compare",
                    FunctionAbi {
                        args: vec![PassMode::Direct],
                        ret: PassMode::Direct,
                        c_variadic: false,
                    },
                    types,
                );
                graph.level = GraphLevel::Specialized;
                graph.start = graph.add_node(
                    Opcode::Start,
                    NodeId::NONE,
                    &[],
                    0,
                    ValueType::Tuple(vec![
                        ValueType::Control,
                        ValueType::Effect,
                        ValueType::Rust(byte),
                    ]),
                );
                let control = graph.add_node(
                    Opcode::Projection,
                    NodeId::NONE,
                    &[graph.start],
                    0,
                    ValueType::Control,
                );
                let effect = graph.add_node(
                    Opcode::Projection,
                    NodeId::NONE,
                    &[graph.start],
                    1,
                    ValueType::Effect,
                );
                let argument = graph.add_node(
                    Opcode::Projection,
                    NodeId::NONE,
                    &[graph.start],
                    2,
                    ValueType::Rust(byte),
                );
                let extension = graph.add_node(
                    Opcode::IntCast,
                    NodeId::NONE,
                    &[argument],
                    0,
                    ValueType::Rust(word),
                );
                let limit = graph.add_constant(17, word);
                let comparison = graph.add_node(
                    Opcode::Compare,
                    NodeId::NONE,
                    &[extension, limit],
                    ComparePredicate::Equal as u32,
                    ValueType::Rust(boolean),
                );
                let condition = match wrapper {
                    0 => comparison,
                    1 => graph.add_node(
                        Opcode::BitNot,
                        NodeId::NONE,
                        &[comparison],
                        0,
                        ValueType::Rust(boolean),
                    ),
                    _ => {
                        let zero = graph.add_constant(0, boolean);
                        graph.add_node(
                            Opcode::Compare,
                            NodeId::NONE,
                            &[comparison, zero],
                            ComparePredicate::Equal as u32,
                            ValueType::Rust(boolean),
                        )
                    }
                };
                let source = graph.add_node(
                    Opcode::MakeVariant,
                    NodeId::NONE,
                    &[argument],
                    0,
                    ValueType::Rust(enumeration),
                );
                let operation = if assumption {
                    graph.add_node(
                        Opcode::Assume,
                        control,
                        &[effect, condition],
                        0,
                        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
                    )
                } else {
                    graph.add_node(
                        Opcode::If,
                        control,
                        &[condition],
                        0,
                        ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
                    )
                };
                let proof = graph.add_node(
                    Opcode::Projection,
                    NodeId::NONE,
                    &[operation],
                    0,
                    ValueType::Control,
                );
                let next_effect = if assumption {
                    graph.add_node(
                        Opcode::Projection,
                        NodeId::NONE,
                        &[operation],
                        1,
                        ValueType::Effect,
                    )
                } else {
                    let no = graph.add_node(
                        Opcode::Projection,
                        NodeId::NONE,
                        &[operation],
                        1,
                        ValueType::Control,
                    );
                    graph.add_node(
                        Opcode::Return,
                        no,
                        &[effect, argument],
                        0,
                        ValueType::Control,
                    );
                    effect
                };
                let witness = graph.add_node(
                    Opcode::VariantWitness,
                    proof,
                    &[source, proof],
                    0,
                    ValueType::Witness,
                );
                graph.variant_fields.push((0, 0));
                let field = graph.add_node(
                    Opcode::VariantField,
                    NodeId::NONE,
                    &[source, witness],
                    0,
                    ValueType::Rust(byte),
                );
                graph.add_node(
                    Opcode::Return,
                    proof,
                    &[next_effect, field],
                    0,
                    ValueType::Control,
                );
                verify_graph(&mut graph).unwrap();
                assert!(combine(&mut graph, comparison).is_none());
                verify_graph(&mut graph).unwrap();
            }
        }
    }
}
