//! SoN verifier (`docs/design/ir/verifier.md`), P0 subset.
//!
//! Structural: inputs exist, arities, `Phi`/`Region` correspondence,
//! projections. Typed: operand and result types per opcode. Full: control
//! reachability and dominance — every value's earliest placement must
//! dominate each use (a `Phi` operand: the matching predecessor edge). The
//! dominance check is what rejects the T01 miscompile shape, where a value
//! defined in one branch is used after the merge.

use std::collections::{HashMap, HashSet};

use kfrc_body::{Mutability, PassMode, TypeOracle, TypeShape};

use crate::analysis::{
    cfg::{ControlFlow, is_anchored_value, is_control_flow},
    dominators::Dominators,
    memory::MemoryFacts,
};
use crate::graph::{
    ArgPermission, FunctionGraph, GraphLevel, GraphState, Hole, MemoryPartition, NodeId, Opcode, ValueType,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyError {
    DanglingInput {
        node: NodeId,
        input: usize,
    },
    WrongArity {
        node: NodeId,
        expected: String,
        actual: usize,
    },
    MissingControl {
        node: NodeId,
    },
    PhiArity {
        phi: NodeId,
        region: NodeId,
    },
    BadRegionInput {
        region: NodeId,
        input: NodeId,
    },
    BadProjection {
        node: NodeId,
    },
    BadPayload {
        node: NodeId,
        detail: String,
    },
    TypeMismatch {
        node: NodeId,
        detail: String,
    },
    Unreachable {
        node: NodeId,
    },
    DominanceViolation {
        definition: NodeId,
        user: NodeId,
    },
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "graph verification failed: {self:?}")
    }
}

impl std::error::Error for VerifyError {}

/// Runs all checks; on success marks the graph `Verified`.
pub fn verify_graph(graph: &mut FunctionGraph) -> Result<(), VerifyError> {
    graph.invalidate();
    check(graph)?;
    graph.state = GraphState::Verified;
    Ok(())
}

pub fn check(graph: &FunctionGraph) -> Result<(), VerifyError> {
    structural(graph)?;
    stage(graph)?;
    typed(graph)?;
    permissions(graph)?;
    dominance(graph)
}

/// RS5.1: permissions describe S reference parameters only. R has not chosen
/// an instance and L has erased them at the layout boundary.
fn permissions(graph: &FunctionGraph) -> Result<(), VerifyError> {
    let any_argument = graph
        .arg_permissions
        .iter()
        .any(|permission| *permission != ArgPermission::None);
    if graph.level != GraphLevel::Specialized {
        if any_argument || !graph.callee_permissions.is_empty() {
            return Err(mismatch(
                graph.start,
                "only Specialized graphs carry argument permissions",
            ));
        }
        return Ok(());
    }
    let ValueType::Tuple(start) = graph.value_type(graph.start) else {
        return Err(mismatch(graph.start, "Start must produce a tuple"));
    };
    let parameters = start.len().saturating_sub(2);
    if !graph.arg_permissions.is_empty() && graph.arg_permissions.len() != parameters {
        return Err(mismatch(
            graph.start,
            format!(
                "{} argument permissions for {parameters} parameters",
                graph.arg_permissions.len()
            ),
        ));
    }
    for (parameter, permission) in graph.arg_permissions.iter().enumerate() {
        let required = match permission {
            ArgPermission::None => continue,
            ArgPermission::Frozen { .. } => Mutability::Not,
            ArgPermission::Unique { .. } => Mutability::Mut,
        };
        let reference = match &start[parameter + 2] {
            ValueType::Rust(ty) => match graph.oracle.semantic().shape(*ty) {
                TypeShape::Reference { mutability, .. } => Some(*mutability),
                _ => None,
            },
            _ => None,
        };
        if reference != Some(required) {
            return Err(mismatch(
                graph.start,
                format!("parameter {parameter} permission {permission:?} needs a {required:?} reference"),
            ));
        }
    }
    for target in &graph.callees {
        if let Some(permissions) = graph.callee_permissions.get(&target.symbol)
            && permissions.len() != target.abi.args.len()
        {
            return Err(mismatch(
                graph.start,
                format!(
                    "{} call permissions for {} arguments of `{}`",
                    permissions.len(),
                    target.abi.args.len(),
                    target.symbol
                ),
            ));
        }
    }
    Ok(())
}

fn arity(node: NodeId, expected: &str, actual: usize, ok: bool) -> Result<(), VerifyError> {
    if ok {
        Ok(())
    } else {
        Err(VerifyError::WrongArity {
            node,
            expected: expected.to_owned(),
            actual,
        })
    }
}

fn structural(graph: &FunctionGraph) -> Result<(), VerifyError> {
    if graph.start.is_none()
        || graph.start.index() >= graph.len()
        || graph.is_dead(graph.start)
        || graph.op(graph.start) != Opcode::Start
    {
        return Err(mismatch(graph.start, "graph must name a live Start node"));
    }
    for mode in graph
        .abi
        .args
        .iter()
        .chain(std::iter::once(&graph.abi.ret))
        .chain(graph.callees.iter().flat_map(|target| {
            target
                .abi
                .args
                .iter()
                .chain(std::iter::once(&target.abi.ret))
        }))
        .chain(
            graph
                .call_abis
                .iter()
                .flat_map(|abi| abi.args.iter().chain(std::iter::once(&abi.ret))),
        )
        .filter(|_| graph.level != GraphLevel::Rust)
    {
        if matches!(mode, PassMode::Vector { bytes } if *bytes != 16) {
            return Err(VerifyError::BadPayload {
                node: graph.start,
                detail: "unsupported SIMD ABI width".to_owned(),
            });
        }
    }
    let count = graph.len();
    for node in graph.ids() {
        let inputs = graph.inputs(node);
        for (index, input) in inputs.iter().enumerate() {
            if input.is_none() || input.index() >= count || graph.is_dead(*input) {
                return Err(VerifyError::DanglingInput { node, input: index });
            }
            if graph.value_type(*input) == &ValueType::Witness
                && !(graph.op(node) == Opcode::VariantField && index == 1)
            {
                return Err(mismatch(
                    node,
                    "witness may only be consumed as a VariantField proof",
                ));
            }
        }
        let control = graph.control(node);
        if !control.is_none() && (control.index() >= count || graph.is_dead(control)) {
            return Err(VerifyError::MissingControl { node });
        }
        let op = graph.op(node);
        let payload_ok = match op {
            Opcode::Const | Opcode::ConstRepr => {
                (graph.payload(node) as usize) < graph.constants.len()
            }
            Opcode::Call => (graph.payload(node) as usize) < graph.callees.len(),
            Opcode::VariantSwitch => (graph.payload(node) as usize) < graph.variant_switches.len(),
            Opcode::Hole => (graph.payload(node) as usize) < graph.holes.len(),
            Opcode::TraitCall => (graph.payload(node) as usize) < graph.trait_calls.len(),
            Opcode::CallIndirect => (graph.payload(node) as usize) < graph.call_abis.len(),
            Opcode::StackSlot => graph
                .stack_slots
                .get(graph.payload(node) as usize)
                .is_some_and(|layout| layout.align.is_power_of_two()),
            Opcode::SymbolAddress | Opcode::ThreadLocalAddress => {
                (graph.payload(node) as usize) < graph.symbols.len()
                    && (op != Opcode::ThreadLocalAddress
                        || graph.symbols[graph.payload(node) as usize].1 == 0)
            }
            Opcode::VariantField | Opcode::VariantFieldAddr => {
                (graph.payload(node) as usize) < graph.variant_fields.len()
            }
            Opcode::Load
            | Opcode::Store
            | Opcode::LoadValue
            | Opcode::StoreValue
            | Opcode::SetVariant
            | Opcode::SlotLive
            | Opcode::SlotDead
            | Opcode::CopyNonOverlapping => {
                (graph.payload(node) as usize) < graph.memory_partitions.len()
            }
            Opcode::Repeat | Opcode::RequireDefined | Opcode::Assume => graph.payload(node) == 0,
            Opcode::Compare => {
                (graph.payload(node) as usize) < crate::graph::ComparePredicate::ALL.len()
            }
            op if op.integer_intrinsic_arity().is_some() => {
                crate::IntegerIntrinsicPayload::valid(op, graph.payload(node))
            }
            op if crate::ArithmeticFlags::allowed(op).is_some() => {
                crate::ArithmeticFlags(graph.payload(node)).valid_for(op)
            }
            _ => true,
        };
        if !payload_ok {
            return Err(VerifyError::BadPayload {
                node,
                detail: format!("invalid {op:?} payload {}", graph.payload(node)),
            });
        }
        let needs_control = matches!(
            op,
            Opcode::Jump
                | Opcode::If
                | Opcode::VariantSwitch
                | Opcode::VariantWitness
                | Opcode::Assume
                | Opcode::TraitCall
                | Opcode::Return
                | Opcode::Unreachable
                | Opcode::Call
                | Opcode::CallIndirect
                | Opcode::Opaque
                | Opcode::ThreadLocalAddress
                | Opcode::Load
                | Opcode::Store
                | Opcode::DivSigned
                | Opcode::DivUnsigned
                | Opcode::RemSigned
                | Opcode::RemUnsigned
                | Opcode::LoadValue
                | Opcode::StoreValue
                | Opcode::SetVariant
                | Opcode::SlotLive
                | Opcode::SlotDead
                | Opcode::CopyNonOverlapping
                | Opcode::RequireDefined
                | Opcode::TransmuteValue
        );
        if needs_control && control.is_none() {
            return Err(VerifyError::MissingControl { node });
        }
        let n = inputs.len();
        match op {
            Opcode::Start
            | Opcode::Const
            | Opcode::ConstRepr
            | Opcode::Hole
            | Opcode::Undefined
            | Opcode::Jump
            | Opcode::Unreachable
            | Opcode::StackSlot
            | Opcode::LocalSlot
            | Opcode::SymbolAddress => arity(node, "0", n, n == 0)?,
            Opcode::Projection => {
                arity(node, "1", n, n == 1)?;
                let source = inputs[0];
                let limit = match graph.op(source) {
                    Opcode::Start => 2 + parameter_count(graph) as u32,
                    Opcode::VariantSwitch => graph
                        .variant_switches
                        .get(graph.payload(source) as usize)
                        .map_or(0, |cases| cases.len() as u32 + 1),
                    Opcode::Hole => 2,
                    Opcode::If => 2,
                    Opcode::Call
                    | Opcode::TraitCall
                    | Opcode::CallIndirect
                    | Opcode::Load
                    | Opcode::Opaque
                    | Opcode::ThreadLocalAddress
                    | Opcode::LoadValue
                    | Opcode::TransmuteValue => 3,
                    Opcode::Store
                    | Opcode::StoreValue
                    | Opcode::SetVariant
                    | Opcode::SlotLive
                    | Opcode::SlotDead
                    | Opcode::CopyNonOverlapping
                    | Opcode::RequireDefined
                    | Opcode::Assume => 2,
                    _ => 0,
                };
                if graph.payload(node) >= limit {
                    return Err(VerifyError::BadProjection { node });
                }
            }
            Opcode::Region => {
                arity(node, ">= 1", n, n >= 1)?;
                for input in inputs {
                    if graph.op(*input) != Opcode::Jump {
                        return Err(VerifyError::BadRegionInput {
                            region: node,
                            input: *input,
                        });
                    }
                }
            }
            Opcode::Phi => {
                arity(node, ">= 2", n, n >= 2)?;
                let region = inputs[0];
                if graph.op(region) != Opcode::Region || graph.inputs(region).len() != n - 1 {
                    return Err(VerifyError::PhiArity { phi: node, region });
                }
            }
            Opcode::VariantSwitch
            | Opcode::Repeat
            | Opcode::Field
            | Opcode::VariantOf
            | Opcode::Discriminant
            | Opcode::FieldAddr
            | Opcode::VariantFieldAddr
            | Opcode::PointerCast => arity(node, "1", n, n == 1)?,
            Opcode::VariantField => arity(node, "1 or 2", n, n == 1 || n == 2)?,
            Opcode::VariantWitness
            | Opcode::Assume
            | Opcode::InsertField
            | Opcode::WidePtr
            | Opcode::IndexAddr
            | Opcode::LoadValue
            | Opcode::SlotLive
            | Opcode::SlotDead
            | Opcode::RequireDefined
            | Opcode::TransmuteValue => arity(node, "2", n, n == 2)?,
            Opcode::StoreValue | Opcode::SetVariant => arity(node, "3", n, n == 3)?,
            Opcode::CopyNonOverlapping => arity(node, "4", n, n == 4)?,
            Opcode::If
            | Opcode::Negate
            | Opcode::BitNot
            | Opcode::IntCast
            | Opcode::ThreadLocalAddress => arity(node, "1", n, n == 1)?,
            Opcode::Return => {
                let expected =
                    if graph.level == GraphLevel::Layout && graph.abi.ret == PassMode::Ignore {
                        1
                    } else {
                        2
                    };
                arity(node, &expected.to_string(), n, n == expected)?
            }
            Opcode::TraitCall => {
                let call = &graph.trait_calls[graph.payload(node) as usize];
                let expected = 1 + call.signature.args.len();
                arity(node, &expected.to_string(), n, n == expected)?;
            }
            Opcode::Call => {
                let callee = &graph.callees[graph.payload(node) as usize];
                let expected = 1 + callee.abi.args.len();
                arity(node, &expected.to_string(), n, n == expected)?
            }
            Opcode::CallIndirect => {
                let abi = &graph.call_abis[graph.payload(node) as usize];
                let expected = 2 + abi.args.len();
                arity(node, &expected.to_string(), n, n == expected)?;
            }
            Opcode::Compare
            | Opcode::PtrOffset
            | Opcode::MulOverflow
            | Opcode::Load
            | Opcode::Opaque => arity(node, "2", n, n == 2)?,
            Opcode::Store => arity(node, "3", n, n == 3)?,
            op if op.integer_intrinsic_arity().is_some() => {
                let expected = op.integer_intrinsic_arity().unwrap();
                arity(node, &expected.to_string(), n, n == expected)?;
            }
            op if op.is_binary_arithmetic() => arity(node, "2", n, n == 2)?,
            _ => {}
        }
    }
    Ok(())
}

fn rust_type(graph: &FunctionGraph, node: NodeId) -> Option<kfrc_body::TyId> {
    graph.rust_type(node)
}

fn verify_integer_intrinsic(graph: &FunctionGraph, node: NodeId) -> Result<(), VerifyError> {
    let inputs = graph.inputs(node);
    let integer = |value| {
        if !matches!(graph.value_type(value), ValueType::Rust(_)) {
            return None;
        }
        let ty = graph.rust_type(value)?;
        match graph.oracle.shape(ty) {
            TypeShape::Int { width, signed } if matches!(width.bits(64), 8 | 16 | 32 | 64) => {
                Some((ty, width.bits(64), *signed))
            }
            _ => None,
        }
    };
    let source = integer(inputs[0])
        .ok_or_else(|| mismatch(node, "integer intrinsic requires an 8/16/32/64-bit integer"))?;
    let result = integer(node)
        .ok_or_else(|| mismatch(node, "integer intrinsic requires an integer result"))?;
    let valid = match graph.op(node) {
        Opcode::BitCount => result.1 == 32 && !result.2,
        Opcode::ByteSwap | Opcode::BitReverse => result.0 == source.0,
        Opcode::SaturatingAdd | Opcode::SaturatingSub => {
            result.0 == source.0 && integer(inputs[1]) == Some(source)
        }
        Opcode::FunnelShift => {
            result.0 == source.0
                && integer(inputs[1]) == Some(source)
                && integer(inputs[2]).is_some_and(|(_, _, signed)| !signed)
        }
        _ => unreachable!(),
    };
    if !valid {
        return Err(mismatch(
            node,
            "integer intrinsic operand/result types disagree",
        ));
    }
    Ok(())
}

fn mismatch(node: NodeId, detail: impl Into<String>) -> VerifyError {
    VerifyError::TypeMismatch {
        node,
        detail: detail.into(),
    }
}

/// Receipts are checked against the original graph before the batch mutates
/// conditions. They never survive compaction or an unrelated CFG rewrite.
#[cfg(debug_assertions)]
pub(crate) fn condition_receipts(
    graph: &FunctionGraph,
    receipts: &[crate::analysis::refinement::ConditionReceipt],
) -> Result<(), VerifyError> {
    crate::analysis::refinement::check_conditions(graph, receipts)
        .map_err(|(node, detail)| mismatch(node, detail))
}

fn parameter_count(graph: &FunctionGraph) -> usize {
    if graph.level == GraphLevel::Rust {
        graph
            .rust_signature
            .as_ref()
            .map_or(0, |signature| signature.args.len())
    } else {
        graph.abi.args.len()
    }
}

fn typed_rust_node(graph: &FunctionGraph, node: NodeId) -> Result<(), VerifyError> {
    let signature = graph
        .rust_signature
        .as_ref()
        .ok_or_else(|| mismatch(node, "R signature missing"))?;
    let inputs = graph.inputs(node);
    match graph.op(node) {
        Opcode::Start => {
            let mut expected = vec![ValueType::Control, ValueType::Effect];
            expected.extend(signature.args.iter().copied().map(ValueType::Rust));
            if node != graph.start || graph.value_type(node) != &ValueType::Tuple(expected) {
                return Err(mismatch(node, "R Start differs from its source signature"));
            }
        }
        Opcode::Return => {
            if graph.value_type(inputs[0]) != &ValueType::Effect
                || graph.value_type(inputs[1]) != &ValueType::Rust(signature.ret)
            {
                return Err(mismatch(node, "R Return differs from its source signature"));
            }
        }
        Opcode::Hole => match &graph.holes[graph.payload(node) as usize] {
            Hole::SizeAlign(ty) | Hole::NeedsDrop(ty)
                if ty.0 as usize >= graph.oracle.table.len() =>
            {
                return Err(mismatch(
                    node,
                    "hole references a type absent from its R definition",
                ));
            }
            Hole::SizeAlign(_) => {
                let ValueType::Tuple(results) = graph.value_type(node) else {
                    return Err(mismatch(node, "SizeAlign hole returns size and alignment"));
                };
                if results.len() != 2 || results.iter().any(|result| !matches!(result, ValueType::Rust(ty) if matches!(graph.oracle.shape(*ty), TypeShape::Int { width: kfrc_body::IntegerWidth::Pointer, signed: false }))) { return Err(mismatch(node, "SizeAlign results must both be usize")); }
            }
            Hole::NeedsDrop(_) => {
                if !graph
                    .rust_type(node)
                    .is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Bool))
                {
                    return Err(mismatch(node, "NeedsDrop hole returns bool"));
                }
            }
            Hole::Resolve { .. } | Hole::VariantEncoding(_) => {
                return Err(mismatch(node, "metadata holes cannot be value nodes"));
            }
        },
        Opcode::TraitCall => {
            let call = &graph.trait_calls[graph.payload(node) as usize];
            if !matches!(
                graph.holes.get(call.resolve as usize),
                Some(Hole::Resolve { args, .. }) if args.iter().all(|ty| (ty.0 as usize) < graph.oracle.table.len())
            ) {
                return Err(mismatch(node, "TraitCall needs a Resolve hole"));
            }
            if graph.value_type(inputs[0]) != &ValueType::Effect
                || inputs[1..]
                    .iter()
                    .zip(&call.signature.args)
                    .any(|(value, ty)| graph.value_type(*value) != &ValueType::Rust(*ty))
            {
                return Err(mismatch(
                    node,
                    "TraitCall arguments differ from its source signature",
                ));
            }
            if graph.value_type(node)
                != &ValueType::Tuple(vec![
                    ValueType::Control,
                    ValueType::Effect,
                    ValueType::Rust(call.signature.ret),
                ])
            {
                return Err(mismatch(
                    node,
                    "TraitCall result differs from its source signature",
                ));
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn type_has_param(
    graph: &FunctionGraph,
    ty: kfrc_body::TyId,
    seen: &mut HashSet<kfrc_body::TyId>,
) -> bool {
    if !seen.insert(ty) {
        return false;
    }
    match graph.oracle.shape(ty) {
        TypeShape::Param { .. } => true,
        TypeShape::Tuple(fields) => fields.iter().any(|ty| type_has_param(graph, *ty, seen)),
        TypeShape::Array { element, .. } | TypeShape::Slice { element } => {
            type_has_param(graph, *element, seen)
        }
        TypeShape::Reference { pointee, .. }
        | TypeShape::RawPointer { pointee, .. }
        | TypeShape::PlaceAddress { pointee } => type_has_param(graph, *pointee, seen),
        _ => graph
            .oracle
            .semantic_aggregate(ty)
            .is_some_and(|aggregate| {
                aggregate
                    .fields
                    .iter()
                    .chain(aggregate.variants.iter().flatten())
                    .any(|ty| type_has_param(graph, *ty, seen))
            }),
    }
}

fn root_type_has_param(
    graph: &FunctionGraph,
    ty: kfrc_body::TyId,
    roots: &mut HashMap<kfrc_body::TyId, bool>,
) -> bool {
    // Cache only complete root queries. A recursive nominal cycle can hide a
    // parameter reachable through a different edge of an ancestor, so an
    // intermediate false answer must never become a reusable type fact.
    *roots
        .entry(ty)
        .or_insert_with(|| type_has_param(graph, ty, &mut HashSet::new()))
}

fn typed(graph: &FunctionGraph) -> Result<(), VerifyError> {
    let memory = MemoryFacts::new(graph);
    let tls_symbols: HashSet<_> = graph
        .ids()
        .filter(|node| graph.op(*node) == Opcode::ThreadLocalAddress)
        .map(|node| graph.symbols[graph.payload(node) as usize].0.as_str())
        .chain(
            graph
                .data_objects
                .iter()
                .filter(|object| object.thread_local)
                .map(|object| object.symbol.as_str()),
        )
        .collect();
    if let Some(object) = graph.data_objects.iter().find(|object| {
        object
            .relocations
            .iter()
            .any(|relocation| tls_symbols.contains(relocation.target.as_str()))
    }) {
        return Err(VerifyError::BadPayload {
            node: graph.start,
            detail: format!(
                "ordinary data relocation in {} cannot target TLS",
                object.symbol
            ),
        });
    }
    let is_integer_like = |node: NodeId| {
        !matches!(graph.value_type(node), ValueType::LayoutValue(_))
            && rust_type(graph, node).is_some_and(|ty| {
                if matches!(graph.level, GraphLevel::Rust | GraphLevel::Specialized)
                    && graph.oracle.semantic().is_product(ty)
                {
                    return false;
                }
                matches!(
                    graph.oracle.shape(ty),
                    TypeShape::Int { .. }
                        | TypeShape::Bool
                        | TypeShape::Char
                        | TypeShape::Reference { .. }
                        | TypeShape::RawPointer { .. }
                        | TypeShape::PlaceAddress { .. }
                )
            })
    };
    let is_address = |node: NodeId| {
        if matches!(graph.value_type(node), ValueType::LayoutValue(_)) {
            return graph.level == GraphLevel::Layout;
        }
        rust_type(graph, node).is_some_and(|ty| match graph.oracle.shape(ty) {
            TypeShape::RawPointer { .. }
            | TypeShape::Reference { .. }
            | TypeShape::PlaceAddress { .. } => graph.oracle.semantic().thin_pointee(ty).is_some(),
            TypeShape::Tuple(_) | TypeShape::Array { .. } | TypeShape::Adt { .. } => false,
            _ => false,
        })
    };
    let scalar_layout = |ty: kfrc_body::TyId| {
        graph
            .oracle
            .size_align(ty)
            .is_some_and(|layout| layout.size <= 8)
    };
    for node in graph.ids() {
        let inputs = graph.inputs(node);
        let control = graph.control(node);
        if !control.is_none() && graph.value_type(control) != &ValueType::Control {
            return Err(mismatch(node, "control input must have Control type"));
        }
        if graph.level == GraphLevel::Rust
            && matches!(
                graph.op(node),
                Opcode::Start | Opcode::Return | Opcode::Hole | Opcode::TraitCall
            )
        {
            typed_rust_node(graph, node)?;
            continue;
        }
        if matches!(graph.level, GraphLevel::Rust | GraphLevel::Specialized) {
            match graph.op(node) {
                Opcode::Call | Opcode::CallIndirect | Opcode::Opaque | Opcode::TransmuteValue => {
                    let ValueType::Tuple(results) = graph.value_type(node) else {
                        return Err(mismatch(
                            node,
                            "fixed value must produce control, effect and Rust value",
                        ));
                    };
                    if results.len() != 3
                        || results[..2] != [ValueType::Control, ValueType::Effect]
                        || !matches!(results[2], ValueType::Rust(_))
                    {
                        return Err(mismatch(
                            node,
                            "fixed value result tuple has invalid semantic types",
                        ));
                    }
                    let skip = if graph.op(node) == Opcode::CallIndirect {
                        2
                    } else {
                        1
                    };
                    if inputs[skip..]
                        .iter()
                        .any(|input| !matches!(graph.value_type(*input), ValueType::Rust(_)))
                    {
                        return Err(mismatch(
                            node,
                            "semantic call/opaque arguments must be Rust values",
                        ));
                    }
                    if graph.op(node) == Opcode::Opaque
                        && graph.value_type(inputs[1]) != &results[2]
                    {
                        return Err(mismatch(node, "Opaque must preserve the value type"));
                    }
                    if graph.op(node) == Opcode::CallIndirect && !is_address(inputs[1]) {
                        return Err(mismatch(node, "indirect callee must be a thin pointer"));
                    }
                }
                Opcode::Return
                    if inputs.len() == 2
                        && !matches!(graph.value_type(inputs[1]), ValueType::Rust(_)) =>
                {
                    return Err(mismatch(node, "semantic return must carry a Rust value"));
                }
                _ => {}
            }
        }
        match graph.op(node) {
            Opcode::RequireDefined | Opcode::Assume => {
                if graph.value_type(inputs[0]) != &ValueType::Effect
                    || !matches!(graph.value_type(inputs[1]), ValueType::Rust(_))
                    || graph.value_type(node)
                        != &ValueType::Tuple(vec![ValueType::Control, ValueType::Effect])
                {
                    return Err(mismatch(
                        node,
                        "RequireDefined consumes effect and Rust value and returns only control and effect",
                    ));
                }
                if graph.op(node) == Opcode::Assume
                    && !graph
                        .rust_type(inputs[1])
                        .is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Bool))
                {
                    return Err(mismatch(node, "Assume requires a bool predicate"));
                }
            }
            Opcode::VariantSwitch => {
                let ty = graph
                    .rust_type(inputs[0])
                    .ok_or_else(|| mismatch(node, "VariantSwitch needs an enum value"))?;
                let count = graph
                    .oracle
                    .semantic()
                    .variant_count(ty)
                    .ok_or_else(|| mismatch(node, "VariantSwitch input is not an enum"))?;
                let cases = &graph.variant_switches[graph.payload(node) as usize];
                let mut seen = HashSet::new();
                if cases
                    .iter()
                    .any(|variant| *variant as usize >= count || !seen.insert(*variant))
                {
                    return Err(mismatch(
                        node,
                        "VariantSwitch cases must be distinct logical variants",
                    ));
                }
                if graph.value_type(node)
                    != &ValueType::Tuple(vec![ValueType::Control; cases.len() + 1])
                {
                    return Err(mismatch(
                        node,
                        "VariantSwitch needs one control per case and otherwise",
                    ));
                }
            }
            Opcode::VariantWitness => {
                let valid = graph
                    .rust_type(inputs[0])
                    .and_then(|ty| graph.oracle.semantic().variant_count(ty))
                    .is_some_and(|count| (graph.payload(node) as usize) < count);
                if !valid
                    || graph.value_type(node) != &ValueType::Witness
                    || graph.value_type(inputs[1]) != &ValueType::Control
                {
                    return Err(mismatch(
                        node,
                        "VariantWitness requires an enum value, valid variant and proof control",
                    ));
                }
            }
            Opcode::LocalSlot
            | Opcode::FieldAddr
            | Opcode::VariantFieldAddr
            | Opcode::IndexAddr
            | Opcode::PointerCast
            | Opcode::LoadValue
            | Opcode::StoreValue
            | Opcode::SetVariant
            | Opcode::SlotLive
            | Opcode::SlotDead
            | Opcode::CopyNonOverlapping => {
                semantic_memory_node(graph, &memory, node)?;
            }
            Opcode::Aggregate
            | Opcode::Repeat
            | Opcode::Field
            | Opcode::InsertField
            | Opcode::MakeVariant
            | Opcode::VariantOf
            | Opcode::VariantField
            | Opcode::WidePtr
            | Opcode::Discriminant => {
                semantic_node(graph, node)?;
            }
            Opcode::Start => {
                let ValueType::Tuple(elements) = graph.value_type(node) else {
                    return Err(mismatch(
                        node,
                        "Start must produce control, effect and parameters",
                    ));
                };
                if node != graph.start
                    || elements.len() != graph.abi.args.len() + 2
                    || elements[..2] != [ValueType::Control, ValueType::Effect]
                    || elements[2..].iter().any(|element| {
                        !matches!(element, ValueType::Rust(_) | ValueType::LayoutValue(_))
                    })
                {
                    return Err(mismatch(
                        node,
                        "Start result signature differs from graph parameters",
                    ));
                }
            }
            Opcode::ConstRepr => {
                let ty = graph
                    .rust_type(node)
                    .ok_or_else(|| mismatch(node, "representation constant must have Rust type"))?;
                let layout = graph
                    .oracle
                    .size_align(ty)
                    .ok_or_else(|| mismatch(node, "representation constant lacks layout"))?;
                if layout.size > 16
                    || (layout.size < 16 && graph.constant_value(node) >> (layout.size * 8) != 0)
                {
                    return Err(mismatch(
                        node,
                        "representation constant exceeds storage width",
                    ));
                }
            }
            Opcode::TransmuteValue => {
                if graph.value_type(inputs[0]) != &ValueType::Effect {
                    return Err(mismatch(node, "transmute requires effect"));
                }
                let source = graph
                    .rust_type(inputs[1])
                    .ok_or_else(|| mismatch(node, "transmute source type missing"))?;
                let ValueType::Tuple(results) = graph.value_type(node) else {
                    return Err(mismatch(node, "transmute result tuple missing"));
                };
                let Some(ValueType::Rust(target)) = results.get(2) else {
                    return Err(mismatch(node, "transmute result type missing"));
                };
                if graph.oracle.size_align(source).map(|layout| layout.size)
                    != graph.oracle.size_align(*target).map(|layout| layout.size)
                    || graph.oracle.size_align(source).is_none()
                {
                    return Err(mismatch(
                        node,
                        "transmute source/target layouts differ in size",
                    ));
                }
            }
            Opcode::Const if matches!(graph.level, GraphLevel::Rust | GraphLevel::Specialized) => {
                let ty = rust_type(graph, node)
                    .ok_or_else(|| mismatch(node, "constant requires Rust type"))?;
                if !semantic_scalar(graph, ty)
                    && !(matches!(graph.oracle.semantic().shape(ty), TypeShape::Unit)
                        && graph.constant_value(node) == 0)
                {
                    return Err(mismatch(
                        node,
                        "semantic aggregate constants require value constructors",
                    ));
                }
            }
            Opcode::Projection => {
                let ValueType::Tuple(elements) = graph.value_type(inputs[0]) else {
                    return Err(mismatch(node, "projection source must be a tuple"));
                };
                if elements.get(graph.payload(node) as usize) != Some(graph.value_type(node)) {
                    return Err(mismatch(node, "projection type differs from source result"));
                }
            }
            Opcode::StackSlot | Opcode::SymbolAddress => {
                if graph.op(node) == Opcode::SymbolAddress {
                    let (symbol, _) = &graph.symbols[graph.payload(node) as usize];
                    if tls_symbols.contains(symbol.as_str()) {
                        return Err(mismatch(node, "TLS symbol requires ThreadLocalAddress"));
                    }
                }
                if !is_address(node) {
                    return Err(mismatch(node, "address node must produce an address type"));
                }
            }
            Opcode::PtrOffset => {
                if !is_address(inputs[0]) || !is_integer_like(inputs[1]) || !is_address(node) {
                    return Err(mismatch(
                        node,
                        "pointer offset requires an address and integer offset",
                    ));
                }
            }
            op if op.integer_intrinsic_arity().is_some() => {
                verify_integer_intrinsic(graph, node)?;
            }
            Opcode::MulOverflow => {
                if !is_integer_like(inputs[0])
                    || rust_type(graph, inputs[0]) != rust_type(graph, inputs[1])
                    || !rust_type(graph, node)
                        .is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Bool))
                {
                    return Err(mismatch(
                        node,
                        "overflow multiplication requires matching integer operands and bool result",
                    ));
                }
            }
            Opcode::Load => {
                verify_partition(graph, &memory, node)?;
                if *graph.value_type(inputs[0]) != ValueType::Effect || !is_address(inputs[1]) {
                    return Err(mismatch(node, "load requires effect and address"));
                }
                let ValueType::Tuple(elements) = graph.value_type(node) else {
                    return Err(mismatch(node, "load must produce control, effect, value"));
                };
                if elements.len() != 3
                    || elements[..2] != [ValueType::Control, ValueType::Effect]
                    || !matches!(elements[2],ValueType::Rust(ty) if scalar_layout(ty))
                {
                    return Err(mismatch(
                        node,
                        "load must produce control, effect, a sized scalar",
                    ));
                }
            }
            Opcode::ThreadLocalAddress => {
                let symbol = &graph.symbols[graph.payload(node) as usize].0;
                if graph
                    .data_objects
                    .iter()
                    .any(|object| object.symbol == *symbol && !object.thread_local)
                {
                    return Err(mismatch(node, "TLS address names a global data object"));
                }
                let ValueType::Tuple(elements) = graph.value_type(node) else {
                    return Err(mismatch(
                        node,
                        "TLS address must produce control, effect, pointer",
                    ));
                };
                if *graph.value_type(inputs[0]) != ValueType::Effect
                    || elements.len() != 3
                    || elements[..2] != [ValueType::Control, ValueType::Effect]
                    || !matches!(elements[2], ValueType::Rust(ty)
                        if matches!(graph.oracle.shape(ty), TypeShape::RawPointer { .. } | TypeShape::Reference { .. } | TypeShape::PlaceAddress { .. })
                            && graph.oracle.size_align(ty).is_some_and(|layout| layout.size == 8))
                {
                    return Err(mismatch(
                        node,
                        "TLS address requires effect and produces control, effect, thin pointer",
                    ));
                }
            }
            Opcode::Store => {
                verify_partition(graph, &memory, node)?;
                if *graph.value_type(inputs[0]) != ValueType::Effect
                    || !is_address(inputs[1])
                    || !rust_type(graph, inputs[2]).is_some_and(scalar_layout)
                    || *graph.value_type(node)
                        != ValueType::Tuple(vec![ValueType::Control, ValueType::Effect])
                {
                    return Err(mismatch(
                        node,
                        "store requires effect, address, sized scalar and produces control/effect",
                    ));
                }
            }
            op if op.is_binary_arithmetic() => {
                let shift = matches!(
                    op,
                    Opcode::ShiftLeft | Opcode::ShiftRightArithmetic | Opcode::ShiftRightLogical
                );
                let result = rust_type(graph, node);
                if rust_type(graph, inputs[0]) != result
                    || (!shift && rust_type(graph, inputs[1]) != result)
                {
                    return Err(mismatch(node, "binary operands must have the result type"));
                }
                if !is_integer_like(inputs[0]) || !is_integer_like(inputs[1]) {
                    return Err(mismatch(node, "binary operands must be integers or bool"));
                }
            }
            Opcode::Negate | Opcode::BitNot => {
                if rust_type(graph, inputs[0]) != rust_type(graph, node)
                    || !is_integer_like(inputs[0])
                {
                    return Err(mismatch(node, "unary operand must have the result type"));
                }
            }
            Opcode::IntCast => {
                if !is_integer_like(inputs[0]) || !is_integer_like(node) {
                    return Err(mismatch(node, "IntToInt casts integers"));
                }
            }
            Opcode::Compare => {
                if matches!(graph.level, GraphLevel::Rust | GraphLevel::Specialized)
                    && (!is_integer_like(inputs[0]) || !is_integer_like(inputs[1]))
                {
                    return Err(mismatch(
                        node,
                        "semantic comparison requires scalar operands",
                    ));
                }
                if rust_type(graph, inputs[0]) != rust_type(graph, inputs[1]) {
                    return Err(mismatch(node, "compared values must have the same type"));
                }
                if !rust_type(graph, node)
                    .is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Bool))
                {
                    return Err(mismatch(node, "comparison result must be bool"));
                }
            }
            Opcode::If => {
                if !rust_type(graph, inputs[0])
                    .is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Bool))
                {
                    return Err(mismatch(node, "branch condition must be bool"));
                }
            }
            Opcode::Phi => {
                let ty = graph.value_type(node);
                for operand in &inputs[1..] {
                    if graph.value_type(*operand) != ty {
                        return Err(mismatch(
                            node,
                            format!("phi operand {operand} has a different type"),
                        ));
                    }
                }
            }
            Opcode::Return | Opcode::Call | Opcode::CallIndirect | Opcode::Opaque => {
                let is_vector_value = |value: NodeId| {
                    rust_type(graph, value).is_some_and(|ty| {
                        graph
                            .oracle
                            .size_align(ty)
                            .is_some_and(|layout| layout.size == 16)
                    })
                };
                if graph.op(node) == Opcode::Return
                    && matches!(graph.abi.ret, PassMode::Vector { .. })
                    && !is_vector_value(inputs[1])
                {
                    return Err(mismatch(
                        node,
                        "SIMD return must have a 16-byte representation",
                    ));
                }
                let call = match graph.op(node) {
                    Opcode::Call => Some((&graph.callees[graph.payload(node) as usize].abi, 1)),
                    Opcode::CallIndirect => {
                        Some((&graph.call_abis[graph.payload(node) as usize], 2))
                    }
                    _ => None,
                };
                if let Some((abi, skip)) = call {
                    if abi.args.iter().zip(&inputs[skip..]).any(|(mode, value)| {
                        matches!(mode, PassMode::Vector { .. }) && !is_vector_value(*value)
                    }) {
                        return Err(mismatch(
                            node,
                            "SIMD call argument must have a 16-byte representation",
                        ));
                    }
                    if matches!(abi.ret, PassMode::Vector { .. })
                        && !matches!(graph.value_type(node), ValueType::Tuple(elements)
                            if matches!(elements.get(2), Some(ValueType::Rust(ty))
                                if graph.oracle.size_align(*ty).is_some_and(|layout| layout.size == 16)))
                    {
                        return Err(mismatch(
                            node,
                            "SIMD call result must have a 16-byte representation",
                        ));
                    }
                }
                if *graph.value_type(inputs[0]) != ValueType::Effect {
                    return Err(mismatch(node, "first input must be the effect token"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn semantic_scalar(graph: &FunctionGraph, ty: kfrc_body::TyId) -> bool {
    let semantic = graph.oracle.semantic();
    !semantic.is_enum(ty)
        && !semantic.is_product(ty)
        && matches!(
            semantic.shape(ty),
            TypeShape::Bool
                | TypeShape::Char
                | TypeShape::Int { .. }
                | TypeShape::Reference { .. }
                | TypeShape::RawPointer { .. }
                | TypeShape::PlaceAddress { .. }
                | TypeShape::FunctionItem { .. }
        )
}

fn stage(graph: &FunctionGraph) -> Result<(), VerifyError> {
    if (graph.level == GraphLevel::Rust) != graph.rust_signature.is_some() {
        return Err(mismatch(
            graph.start,
            "only R graphs require a source Rust signature",
        ));
    }
    fn value_type(
        graph: &FunctionGraph,
        node: NodeId,
        value: &ValueType,
        type_parameters: &mut HashMap<kfrc_body::TyId, bool>,
    ) -> Result<(), VerifyError> {
        match value {
            ValueType::Witness => {
                if graph.level == GraphLevel::Layout
                    || graph.op(node) != Opcode::VariantWitness
                    || graph.value_type(node) != &ValueType::Witness
                {
                    return Err(mismatch(
                        node,
                        "Witness is only the R/S result of VariantWitness",
                    ));
                }
            }
            ValueType::Tuple(elements) => {
                for element in elements {
                    value_type(graph, node, element, type_parameters)?;
                }
            }
            ValueType::LayoutValue(ty) => {
                if ty.0 as usize >= graph.oracle.table.len() {
                    return Err(mismatch(node, "value type is absent from the type table"));
                }
                if root_type_has_param(graph, *ty, type_parameters) {
                    return Err(mismatch(node, "layout value retains a type parameter"));
                }
                if graph.level != GraphLevel::Layout {
                    return Err(mismatch(
                        node,
                        "LayoutValue is only legal after layout lowering",
                    ));
                }
                if graph.oracle.size_align(*ty).is_none() {
                    return Err(mismatch(
                        node,
                        "LayoutValue lacks a physical storage layout",
                    ));
                }
            }
            ValueType::Rust(ty) => {
                if ty.0 as usize >= graph.oracle.table.len() {
                    return Err(mismatch(node, "value type is absent from the type table"));
                }
                if graph.level != GraphLevel::Rust
                    && root_type_has_param(graph, *ty, type_parameters)
                {
                    return Err(mismatch(node, "type parameter remains outside R"));
                }
                let semantic = graph.oracle.semantic();
                if semantic.is_unsized(*ty) {
                    return Err(mismatch(
                        node,
                        "unsized types require a place, not a by-value result",
                    ));
                }
                match semantic.shape(*ty) {
                    TypeShape::PlaceAddress { pointee } => {
                        if pointee.0 as usize >= graph.oracle.table.len()
                            || semantic.is_product(*ty)
                            || graph.oracle.size_align(*ty)
                                != Some(kfrc_body::SizeAlign { size: 8, align: 8 })
                        {
                            return Err(mismatch(
                                node,
                                "place address requires a thin pointer representation",
                            ));
                        }
                    }
                    TypeShape::Reference { pointee, .. }
                    | TypeShape::RawPointer { pointee, .. }
                        if semantic.requires_pointer_metadata(*pointee) =>
                    {
                        if semantic.field_count(*ty, None) != Some(2)
                            || graph
                                .oracle
                                .size_align(*ty)
                                .is_some_and(|layout| layout.size != 16)
                        {
                            return Err(mismatch(
                                node,
                                "Rust pointer to DST requires genuine wide-pointer metadata",
                            ));
                        }
                    }
                    _ => {}
                }
                if let Some(inner) = semantic.maybe_uninit_inner(*ty) {
                    if inner.0 as usize >= graph.oracle.table.len()
                        || graph.oracle.size_align(*ty).is_none()
                        || graph.oracle.size_align(*ty) != graph.oracle.size_align(inner)
                    {
                        return Err(mismatch(
                            node,
                            "MaybeUninit representation must match its inner type size and alignment",
                        ));
                    }
                }
                if graph.level == GraphLevel::Layout
                    && (semantic.is_enum(*ty)
                        || semantic.maybe_uninit_inner(*ty).is_some()
                        || (semantic.is_product(*ty)
                            && !matches!(semantic.shape(*ty), TypeShape::Unit)))
                {
                    return Err(mismatch(
                        node,
                        "L graph retains a Rust aggregate value instead of LayoutValue",
                    ));
                }
                if matches!(graph.level, GraphLevel::Rust | GraphLevel::Specialized) {
                    if (semantic.is_union(*ty)
                        && (graph.level == GraphLevel::Rust
                            || semantic.maybe_uninit_inner(*ty).is_none()))
                        || semantic.is_vector(*ty)
                        || matches!(
                            semantic.shape(*ty),
                            TypeShape::Float { .. }
                                | TypeShape::Int {
                                    width: kfrc_body::IntegerWidth::W128,
                                    ..
                                }
                                | TypeShape::Other { .. }
                                | TypeShape::Slice { .. }
                                | TypeShape::Str
                        )
                    {
                        return Err(mismatch(
                            node,
                            "type is outside the specialized value graph subset",
                        ));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    // This memo belongs to one read-only verification, never to GraphState:
    // later oracle or graph edits must be checked from scratch.
    let mut type_parameters = HashMap::new();
    for node in graph.ids() {
        let op = graph.op(node);
        if graph.level != GraphLevel::Rust && matches!(op, Opcode::Hole | Opcode::TraitCall) {
            return Err(mismatch(
                node,
                "unresolved Hole/TraitCall are only legal in R",
            ));
        }
        if graph.level == GraphLevel::Rust
            && matches!(
                op,
                Opcode::Call | Opcode::CallIndirect | Opcode::ConstRepr | Opcode::TransmuteValue
            )
        {
            return Err(mismatch(
                node,
                "R uses logical Resolve calls and explicit holes for layout questions",
            ));
        }
        if graph.level == GraphLevel::Layout && op == Opcode::VariantSwitch {
            return Err(mismatch(node, "VariantSwitch must be lowered before L"));
        }
        if graph.level == GraphLevel::Layout
            && matches!(
                op,
                Opcode::RequireDefined | Opcode::Assume | Opcode::VariantWitness
            )
        {
            return Err(mismatch(
                node,
                "semantic requirement or witness must be erased before published L",
            ));
        }
        value_type(graph, node, graph.value_type(node), &mut type_parameters)?;
        let op = graph.op(node);
        if (op.is_semantic_memory() || op.is_semantic_value()) && graph.level == GraphLevel::Layout
        {
            return Err(mismatch(
                node,
                "semantic value operation must be lowered before L",
            ));
        }
        if matches!(graph.level, GraphLevel::Rust | GraphLevel::Specialized)
            && matches!(
                op,
                Opcode::StackSlot | Opcode::Load | Opcode::Store | Opcode::ThreadLocalAddress
            )
        {
            return Err(mismatch(
                node,
                "physical memory operations are only legal after layout lowering",
            ));
        }
    }
    Ok(())
}

fn pointer_pointee(graph: &FunctionGraph, node: NodeId) -> Option<kfrc_body::TyId> {
    let ValueType::Rust(ty) = graph.value_type(node) else {
        return None;
    };
    graph.oracle.semantic().thin_pointee(*ty)
}

fn semantic_memory_node(
    graph: &FunctionGraph,
    memory: &MemoryFacts,
    node: NodeId,
) -> Result<(), VerifyError> {
    let inputs = graph.inputs(node);
    let op = graph.op(node);
    if op == Opcode::LocalSlot {
        if pointer_pointee(graph, node).is_none_or(|ty| graph.oracle.semantic().is_unsized(ty)) {
            return Err(mismatch(
                node,
                "LocalSlot requires an address of a sized local",
            ));
        }
        if memory.local_root(graph.payload(node)) != Some(node) {
            return Err(mismatch(node, "LocalSlot storage identity is duplicated"));
        }
        return Ok(());
    }
    if matches!(
        op,
        Opcode::FieldAddr | Opcode::VariantFieldAddr | Opcode::IndexAddr | Opcode::PointerCast
    ) {
        let source = pointer_pointee(graph, inputs[0])
            .ok_or_else(|| mismatch(node, "address operation requires thin pointer"))?;
        let result = pointer_pointee(graph, node)
            .ok_or_else(|| mismatch(node, "address operation must produce thin pointer"))?;
        match op {
            Opcode::PointerCast => {}
            Opcode::IndexAddr => {
                if !graph
                    .rust_type(inputs[1])
                    .is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Int { .. }))
                {
                    return Err(mismatch(node, "IndexAddr requires integer index"));
                }
                // Pointer arithmetic preserves T even when T is itself an
                // array. A place projection instead selects the immediate
                // array/slice element. Both use the result pointee's stride;
                // neither permits arbitrary retyping or an unsized stride.
                let correct_element = !graph.oracle.semantic().is_unsized(result)
                    && (result == source
                        || match graph.oracle.shape(source) {
                            TypeShape::Array { element, .. } | TypeShape::Slice { element } => {
                                result == *element
                            }
                            TypeShape::Str => matches!(
                                graph.oracle.shape(result),
                                TypeShape::Int {
                                    width: kfrc_body::IntegerWidth::W8,
                                    signed: false,
                                }
                            ),
                            _ => false,
                        });
                if !correct_element {
                    return Err(mismatch(
                        node,
                        "IndexAddr requires a sized source pointee or immediate element type",
                    ));
                }
            }
            _ => {
                let (variant, field) = if op == Opcode::VariantFieldAddr {
                    let (variant, field) = graph.variant_fields[graph.payload(node) as usize];
                    (Some(variant), field)
                } else {
                    (None, graph.payload(node))
                };
                if graph.oracle.semantic().field_type(source, variant, field) != Some(result) {
                    return Err(mismatch(
                        node,
                        "field address type differs from logical field",
                    ));
                }
                if graph.oracle.semantic().is_unsized(result)
                    && (variant.is_some()
                        || graph.oracle.semantic().static_slice_tail(source) != Some(field))
                {
                    return Err(mismatch(
                        node,
                        "unsized field address requires a static slice-tail contract",
                    ));
                }
            }
        }
        return Ok(());
    }
    if graph.value_type(inputs[0]) != &ValueType::Effect {
        return Err(mismatch(node, "memory operation requires effect token"));
    }
    let pointee = pointer_pointee(graph, inputs[1])
        .ok_or_else(|| mismatch(node, "memory operation requires a thin pointer"))?;
    if graph.oracle.semantic().is_unsized(pointee) {
        return Err(mismatch(
            node,
            "whole-value memory access requires a sized pointee",
        ));
    }
    let expected = if op == Opcode::LoadValue {
        vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(pointee),
        ]
    } else {
        vec![ValueType::Control, ValueType::Effect]
    };
    if graph.value_type(node) != &ValueType::Tuple(expected) {
        return Err(mismatch(node, "memory operation result types disagree"));
    }
    match op {
        Opcode::StoreValue if graph.value_type(inputs[2]) != &ValueType::Rust(pointee) => {
            return Err(mismatch(
                node,
                "StoreValue destination differs from value type",
            ));
        }
        Opcode::SetVariant => {
            if graph.op(inputs[2]) != Opcode::Const
                || !graph.oracle.semantic().is_enum(pointee)
                || !graph
                    .rust_type(inputs[2])
                    .is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Int { .. }))
            {
                return Err(mismatch(
                    node,
                    "SetVariant requires enum storage and constant logical variant",
                ));
            }
            let variant = u32::try_from(graph.constant_value(inputs[2])).ok();
            if variant
                .and_then(|variant| graph.oracle.semantic().field_types(pointee, Some(variant)))
                .is_none()
            {
                return Err(mismatch(node, "SetVariant logical variant is absent"));
            }
        }
        Opcode::SlotLive | Opcode::SlotDead => {
            if !matches!(graph.op(inputs[1]), Opcode::LocalSlot | Opcode::StackSlot) {
                return Err(mismatch(
                    node,
                    "storage lifetime must target a local allocation root",
                ));
            }
        }
        Opcode::CopyNonOverlapping => {
            if pointer_pointee(graph, inputs[2]) != Some(pointee)
                || !graph
                    .rust_type(inputs[3])
                    .is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Int { .. }))
            {
                return Err(mismatch(
                    node,
                    "copy requires equal element types and integer count",
                ));
            }
        }
        _ => {}
    }
    verify_partition(graph, memory, node)
}

fn verify_partition(
    graph: &FunctionGraph,
    memory: &MemoryFacts,
    node: NodeId,
) -> Result<(), VerifyError> {
    let inputs = graph.inputs(node);
    let op = graph.op(node);
    if op == Opcode::CopyNonOverlapping
        && graph.memory_partitions[graph.payload(node) as usize] != MemoryPartition::Unknown
    {
        return Err(mismatch(
            node,
            "copy touches source and destination and requires Unknown partition",
        ));
    }
    match &graph.memory_partitions[graph.payload(node) as usize] {
        MemoryPartition::Unknown => {}
        MemoryPartition::Const { symbol } => {
            let Some((symbol, _)) = graph.symbols.get(*symbol as usize) else {
                return Err(mismatch(node, "Const partition symbol is absent"));
            };
            if !graph
                .data_objects
                .iter()
                .any(|object| object.symbol == *symbol && !object.writable && !object.thread_local)
            {
                return Err(mismatch(
                    node,
                    "Const partition lacks immutable data evidence",
                ));
            }
            if !matches!(op, Opcode::LoadValue | Opcode::Load) {
                return Err(mismatch(node, "Const partition cannot be written"));
            }
            if !memory.root(inputs[1]).is_some_and(|root| {
                graph.op(root) == Opcode::SymbolAddress
                    && graph.symbols[graph.payload(root) as usize].0 == *symbol
            }) {
                return Err(mismatch(
                    node,
                    "Const load address does not derive from its symbol",
                ));
            }
        }
        MemoryPartition::Local { local } => {
            if memory
                .root(inputs[1])
                .and_then(|root| memory.local_identity(root))
                != Some(*local)
            {
                return Err(mismatch(
                    node,
                    "Local access does not derive from its allocation",
                ));
            }
            if !memory.is_unescaped_local(*local) {
                return Err(mismatch(node, "Local partition allocation escapes"));
            }
        }
    }
    Ok(())
}

fn semantic_node(graph: &FunctionGraph, node: NodeId) -> Result<(), VerifyError> {
    let ty = graph
        .rust_type(node)
        .ok_or_else(|| mismatch(node, "semantic value lacks a Rust type"))?;
    let inputs = graph.inputs(node);
    let semantic = graph.oracle.semantic();
    let check_fields = |fields: &[kfrc_body::TyId]| -> Result<(), VerifyError> {
        if fields.len() != inputs.len() {
            return Err(mismatch(
                node,
                "constructor field count differs from semantic type",
            ));
        }
        for (field, input) in fields.iter().zip(inputs) {
            if graph.value_type(*input) != &ValueType::Rust(*field) {
                return Err(mismatch(
                    node,
                    "constructor field type differs from semantic type",
                ));
            }
        }
        Ok(())
    };
    match graph.op(node) {
        Opcode::Repeat => {
            let TypeShape::Array { element, .. } = semantic.shape(ty) else {
                return Err(mismatch(node, "Repeat result must be an array"));
            };
            if graph.value_type(inputs[0]) != &ValueType::Rust(*element) {
                return Err(mismatch(
                    node,
                    "Repeat operand differs from array element type",
                ));
            }
        }
        Opcode::Aggregate => {
            if semantic.is_enum(ty)
                || semantic.is_union(ty)
                || semantic.is_vector(ty)
                || matches!(
                    semantic.shape(ty),
                    TypeShape::Reference { .. }
                        | TypeShape::RawPointer { .. }
                        | TypeShape::PlaceAddress { .. }
                )
            {
                return Err(mismatch(
                    node,
                    "Aggregate requires an ordinary aggregate type",
                ));
            }
            if let TypeShape::Array { element, length } = semantic.shape(ty) {
                if inputs.len() as u64 != *length
                    || inputs
                        .iter()
                        .any(|input| graph.value_type(*input) != &ValueType::Rust(*element))
                {
                    return Err(mismatch(
                        node,
                        "Aggregate array length or element type differs",
                    ));
                }
            } else {
                let fields = semantic
                    .field_types(ty, None)
                    .ok_or_else(|| mismatch(node, "Aggregate type has no semantic fields"))?;
                check_fields(&fields)?;
            }
        }
        Opcode::WidePtr => {
            if !matches!(
                semantic.shape(ty),
                TypeShape::Reference { .. } | TypeShape::RawPointer { .. }
            ) {
                return Err(mismatch(
                    node,
                    "WidePtr requires a reference or pointer type",
                ));
            }
            let fields = semantic
                .field_types(ty, None)
                .ok_or_else(|| mismatch(node, "wide pointer type lacks semantic components"))?;
            if fields.len() != 2 {
                return Err(mismatch(
                    node,
                    "wide pointer must have data and metadata components",
                ));
            }
            check_fields(&fields)?;
        }
        Opcode::MakeVariant => {
            if !semantic.is_enum(ty) {
                return Err(mismatch(node, "MakeVariant requires enum type"));
            }
            let fields = semantic
                .field_types(ty, Some(graph.payload(node)))
                .ok_or_else(|| {
                    mismatch(node, "constructor variant is absent from semantic enum")
                })?;
            check_fields(&fields)?;
        }
        Opcode::Field | Opcode::InsertField => {
            let source = graph
                .rust_type(inputs[0])
                .ok_or_else(|| mismatch(node, "field source has no Rust type"))?;
            if semantic.is_enum(source) || semantic.is_union(source) || semantic.is_vector(source) {
                return Err(mismatch(
                    node,
                    "Field/InsertField require an ordinary aggregate or wide pointer",
                ));
            }
            let field = semantic
                .field_type(source, None, graph.payload(node))
                .ok_or_else(|| mismatch(node, "field index is absent from semantic type"))?;
            if graph.op(node) == Opcode::Field {
                if ty != field {
                    return Err(mismatch(
                        node,
                        "Field result differs from logical field type",
                    ));
                }
            } else if ty != source || graph.value_type(inputs[1]) != &ValueType::Rust(field) {
                return Err(mismatch(
                    node,
                    "InsertField must preserve aggregate and logical field types",
                ));
            }
        }
        Opcode::VariantField => {
            let source = graph
                .rust_type(inputs[0])
                .ok_or_else(|| mismatch(node, "variant field source lacks Rust type"))?;
            let (variant, field) = graph.variant_fields[graph.payload(node) as usize];
            if !semantic.is_enum(source)
                || semantic.field_type(source, Some(variant), field) != Some(ty)
            {
                return Err(mismatch(
                    node,
                    "VariantField has invalid enum, variant, field or result type",
                ));
            }
            if inputs.len() == 2 {
                let witness = inputs[1];
                if graph.op(witness) != Opcode::VariantWitness
                    || graph.value_type(witness) != &ValueType::Witness
                    || graph.resolve(graph.input(witness, 0)) != graph.resolve(inputs[0])
                    || graph.payload(witness) != variant
                {
                    return Err(mismatch(
                        node,
                        "variant proof must match the exact enum value and variant",
                    ));
                }
            }
        }
        Opcode::VariantOf | Opcode::Discriminant => {
            let source = graph
                .rust_type(inputs[0])
                .ok_or_else(|| mismatch(node, "variant query source lacks Rust type"))?;
            if !semantic.is_enum(source) || !matches!(semantic.shape(ty), TypeShape::Int { .. }) {
                return Err(mismatch(
                    node,
                    "variant query requires enum input and integer result",
                ));
            }
            if graph.op(node) == Opcode::Discriminant {
                let count = semantic
                    .variant_count(source)
                    .ok_or_else(|| mismatch(node, "enum lacks variant definition"))?;
                for variant in 0..count {
                    if semantic.discriminant(source, variant as u32).is_none() {
                        return Err(mismatch(node, "enum lacks source-level discriminants"));
                    }
                }
            }
        }
        _ => unreachable!("semantic opcode checked by caller"),
    }
    Ok(())
}

/// Dominance over the control skeleton, with value placement as early as
/// the inputs allow.
fn dominance(graph: &FunctionGraph) -> Result<(), VerifyError> {
    let cfg = ControlFlow::new(graph);
    let position = &cfg.position;
    for node in graph.ids() {
        if is_control_flow(graph, node) && !cfg.contains(node) {
            return Err(VerifyError::Unreachable { node });
        }
    }
    let dominators = Dominators::new(&cfg);
    crate::analysis::refinement::check(graph, &cfg, &dominators)
        .map_err(|(node, detail)| mismatch(node, detail))?;
    let dominates = |a: usize, b: usize| dominators.dominates_position(a, b);
    // Earliest placement of every node, as a position in the control order.
    let mut placement: HashMap<NodeId, usize> = HashMap::new();
    let ids: Vec<NodeId> = graph.ids().collect();
    let place =
        |node: NodeId, placement: &mut HashMap<NodeId, usize>| -> Result<usize, VerifyError> {
            place_node(graph, node, &position, &dominates, placement)
        };
    for node in &ids {
        place(*node, &mut placement)?;
    }
    // Uses by fixed nodes and phis.
    for node in ids {
        let op = graph.op(node);
        if op == Opcode::Phi {
            let region = graph.input(node, 0);
            let jumps = graph.inputs(region);
            for (slot, operand) in graph.inputs(node)[1..].iter().enumerate() {
                let definition = placement[operand];
                if !dominates(definition, position[&jumps[slot]]) {
                    return Err(VerifyError::DominanceViolation {
                        definition: *operand,
                        user: node,
                    });
                }
            }
        } else if (is_control_flow(graph, node) && op != Opcode::Projection && op != Opcode::Region)
            || is_anchored_value(op)
            || op == Opcode::VariantWitness
        {
            let at = if is_anchored_value(op) || op == Opcode::VariantWitness {
                *position
                    .get(&graph.control(node))
                    .ok_or(VerifyError::Unreachable { node })?
            } else {
                position[&node]
            };
            for input in graph.inputs(node) {
                if !dominates(placement[input], at) {
                    return Err(VerifyError::DominanceViolation {
                        definition: *input,
                        user: node,
                    });
                }
            }
        }
    }
    Ok(())
}

fn place_node(
    graph: &FunctionGraph,
    node: NodeId,
    position: &HashMap<NodeId, usize>,
    dominates: &dyn Fn(usize, usize) -> bool,
    placement: &mut HashMap<NodeId, usize>,
) -> Result<usize, VerifyError> {
    if let Some(at) = placement.get(&node) {
        return Ok(*at);
    }
    let op = graph.op(node);
    let at = if let Some(at) = position.get(&node) {
        *at
    } else {
        match op {
            Opcode::Const
            | Opcode::Undefined
            | Opcode::StackSlot
            | Opcode::LocalSlot
            | Opcode::SymbolAddress => 0,
            op if is_anchored_value(op) => *position
                .get(&graph.control(node))
                .ok_or(VerifyError::Unreachable { node })?,
            Opcode::VariantWitness => *position
                .get(&graph.control(node))
                .ok_or(VerifyError::Unreachable { node })?,
            Opcode::Phi => position[&graph.input(node, 0)],
            Opcode::Projection => {
                // A value result of Start or Call sits where its source sits.
                let source = graph.input(node, 0);
                place_node(graph, source, position, dominates, placement)?
            }
            _ => {
                // Pure: the deepest input placement; inputs must lie on one
                // dominator chain.
                let mut deepest = 0usize;
                let mut deepest_input = NodeId::NONE;
                for input in graph.inputs(node).to_vec() {
                    let input_at = place_node(graph, input, position, dominates, placement)?;
                    if dominates(deepest, input_at) {
                        deepest = input_at;
                        deepest_input = input;
                    } else if !dominates(input_at, deepest) {
                        return Err(VerifyError::DominanceViolation {
                            definition: if deepest_input.is_none() {
                                input
                            } else {
                                deepest_input
                            },
                            user: node,
                        });
                    }
                }
                deepest
            }
        }
    };
    placement.insert(node, at);
    Ok(at)
}
