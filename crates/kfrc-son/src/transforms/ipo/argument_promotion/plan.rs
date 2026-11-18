use std::collections::{BTreeMap, BTreeSet};

use crate::analysis::{
    cfg::ControlFlow,
    uses::{UseKind, Uses},
};
use crate::{FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};
use kfrc_body::{FunctionAbi, PassMode, TyId, TypeOracle, TypeShape};

use super::snapshots;

pub(super) struct Part {
    pub path: Vec<(u32, ValueType)>,
    pub ty: TyId,
    pub loads: Vec<NodeId>,
    pub abi: PassMode,
}

pub(super) struct Argument {
    pub index: usize,
    pub root: NodeId,
    pub addresses: Vec<NodeId>,
    pub parts: Vec<Part>,
    pub protected: bool,
}

pub(super) struct Plan {
    pub arguments: Vec<Argument>,
    pub abi: FunctionAbi,
    pub start: Vec<ValueType>,
}

pub(super) fn build(graph: &FunctionGraph) -> Option<Plan> {
    if graph.level != GraphLevel::Specialized
        || graph.abi.c_variadic
        || graph.live_count() > 2048
        || graph.abi.args.len() > 6
    {
        return None;
    }
    // No writes can change a snapshot between entry and any replacement.
    // Calls and barriers also stop this first version's whole-body proof.
    let whole_body_readonly = !graph.ids().any(|n| {
        matches!(
            graph.op(n),
            Opcode::Call
                | Opcode::CallIndirect
                | Opcode::Opaque
                | Opcode::StoreValue
                | Opcode::SetVariant
                | Opcode::CopyNonOverlapping
                | Opcode::SlotLive
                | Opcode::SlotDead
                | Opcode::TransmuteValue
                | Opcode::ThreadLocalAddress
        )
    });
    if !whole_body_readonly && !snapshots::enabled() {
        return None;
    }
    let ValueType::Tuple(start) = graph.value_type(graph.start) else {
        return None;
    };
    let uses = Uses::new(graph);
    let guaranteed = observed_entry_loads(graph)?;
    let mut arguments = Vec::new();
    for (index, ty) in start[2..].iter().enumerate() {
        if graph.abi.args[index] != PassMode::Direct {
            continue;
        }
        let ValueType::Rust(ty) = *ty else { continue };
        if graph.oracle.semantic().thin_pointee(ty).is_none() {
            continue;
        }
        let roots: Vec<_> = uses
            .of(graph.start)
            .iter()
            .filter_map(|usage| {
                let n = usage.user;
                (graph.op(n) == Opcode::Projection && graph.payload(n) as usize == index + 2)
                    .then_some(n)
            })
            .collect();
        if roots.len() != 1 {
            continue;
        }
        let protected =
            snapshots::enabled() && snapshots::protected_carrier(graph, roots[0], index);
        if !whole_body_readonly && !protected {
            continue;
        }
        if let Some(arg) = argument(graph, &uses, roots[0], index, &guaranteed, protected) {
            arguments.push(arg);
        }
    }
    if arguments.is_empty() {
        return None;
    }
    let mut parameters = vec![ValueType::Control, ValueType::Effect];
    let mut abi = graph.abi.clone();
    abi.args.clear();
    for (index, ty) in start[2..].iter().enumerate() {
        if let Some(arg) = arguments.iter().find(|arg| arg.index == index) {
            for part in &arg.parts {
                parameters.push(ValueType::Rust(part.ty));
                abi.args.push(part.abi);
            }
        } else {
            parameters.push(ty.clone());
            abi.args.push(graph.abi.args[index]);
        }
    }
    if abi.args.len() > 6 {
        return None;
    }
    Some(Plan {
        arguments,
        abi,
        start: parameters,
    })
}

fn argument(
    graph: &FunctionGraph,
    uses: &Uses,
    root: NodeId,
    index: usize,
    guaranteed: &BTreeSet<NodeId>,
    protected: bool,
) -> Option<Argument> {
    let mut parts: BTreeMap<Vec<u32>, Part> = BTreeMap::new();
    let mut addresses = Vec::new();
    let mut stack = vec![(root, Vec::<(u32, ValueType)>::new())];
    let mut visits = 0;
    while let Some((address, path)) = stack.pop() {
        if path.len() > 3 {
            return None;
        }
        for usage in uses.of(address) {
            visits += 1;
            if visits > 256 {
                return None;
            }
            let user = usage.user;
            match (graph.op(user), usage.kind()) {
                (Opcode::PointerCast, UseKind::Operand(0)) if graph.control(user).is_none() => {
                    let from = graph.rust_type(address)?;
                    let to = graph.rust_type(user)?;
                    if graph.oracle.semantic().thin_pointee(from)
                        != graph.oracle.semantic().thin_pointee(to)
                    {
                        return None;
                    }
                    addresses.push(user);
                    stack.push((user, path.clone()));
                }
                (Opcode::FieldAddr, UseKind::Operand(0)) => {
                    let mut next = path.clone();
                    next.push((graph.payload(user), graph.value_type(user).clone()));
                    addresses.push(user);
                    stack.push((user, next));
                }
                (Opcode::LoadValue, UseKind::Operand(1)) => {
                    let ValueType::Tuple(result) = graph.value_type(user) else {
                        return None;
                    };
                    let ValueType::Rust(ty) = result[2] else {
                        return None;
                    };
                    let abi = if scalar(graph, ty) {
                        PassMode::Direct
                    } else {
                        snapshots::transport(graph, ty)?
                    };
                    if uses.of(user).iter().any(|u| {
                        u.kind() != UseKind::Operand(0) || graph.op(u.user) != Opcode::Projection
                    }) {
                        return None;
                    }
                    let key: Vec<_> = path.iter().map(|(field, _)| *field).collect();
                    let part = parts.entry(key).or_insert_with(|| Part {
                        path: path.clone(),
                        ty,
                        loads: Vec::new(),
                        abi,
                    });
                    if part.ty != ty {
                        return None;
                    }
                    part.loads.push(user);
                    if parts.len() > 3 {
                        return None;
                    }
                }
                _ => return None,
            }
        }
    }
    if parts.is_empty()
        || parts
            .values()
            .any(|part| !part.loads.iter().any(|load| guaranteed.contains(load)))
    {
        return None;
    }
    // Whole-object and field reads of overlapping storage are different parts.
    // Only scalar leaves enter the plan, but reject a path prefix explicitly.
    let paths: Vec<_> = parts.keys().collect();
    if paths.windows(2).any(|pair| pair[1].starts_with(pair[0])) {
        return None;
    }
    if protected && !snapshots::valid_accesses(graph, root, parts.values()) {
        return None;
    }
    Some(Argument {
        index,
        root,
        addresses,
        parts: parts.into_values().collect(),
        protected,
    })
}

fn scalar(graph: &FunctionGraph, ty: TyId) -> bool {
    let size = match graph.oracle.shape(ty) {
        TypeShape::Bool => 1,
        TypeShape::Char => 4,
        TypeShape::Int { width, .. } if width.bits(64) <= 64 => u64::from(width.bits(64) / 8),
        _ => return false,
    };
    graph
        .oracle
        .size_align(ty)
        .is_some_and(|layout| layout.size == size)
        && graph.oracle.semantic().maybe_uninit_inner(ty).is_none()
}

/// Passing a new scalar observes poison at the call boundary. Require each
/// promoted load to be both executed and strictly observed before the first
/// branch, return or other effect; an unused/conditional load is insufficient.
fn observed_entry_loads(graph: &FunctionGraph) -> Option<BTreeSet<NodeId>> {
    let cfg = ControlFlow::new(graph);
    let mut control = graph.find_projection(graph.start, 0)?;
    let mut effect = graph.find_projection(graph.start, 1)?;
    let mut prefix = BTreeSet::new();
    let mut observed = BTreeSet::new();
    for _ in 0..64 {
        let [next] = cfg.successors(control) else {
            return None;
        };
        match graph.op(*next) {
            Opcode::Jump => control = *next,
            Opcode::Region if graph.inputs(*next).len() == 1 => {
                control = *next;
                if let Some(phi) = graph.ids().find(|&node| {
                    graph.op(node) == Opcode::Phi
                        && graph.value_type(node) == &ValueType::Effect
                        && graph.inputs(node) == [control, effect]
                }) {
                    effect = phi;
                }
            }
            Opcode::LoadValue if graph.input(*next, 0) == effect => {
                prefix.insert(*next);
                control = graph.find_projection(*next, 0)?;
                effect = graph.find_projection(*next, 1)?;
            }
            Opcode::If
            | Opcode::Return
            | Opcode::RequireDefined
            | Opcode::Assume
            | Opcode::Call
                if graph.op(*next) != Opcode::Call || snapshots::enabled() =>
            {
                let values = if graph.op(*next) == Opcode::Call {
                    // Every whole argument is observed before the call can
                    // execute or fail. A field of a loaded aggregate does not
                    // observe the rest, so Field/WidePtr are not walked below.
                    graph.inputs(*next)[1..].to_vec()
                } else {
                    vec![graph.input(*next, usize::from(graph.op(*next) != Opcode::If))]
                };
                let mut seen = BTreeSet::new();
                let mut stack = values;
                while let Some(value) = stack.pop() {
                    if !seen.insert(value) {
                        continue;
                    }
                    if seen.len() > 512 {
                        return None;
                    }
                    match graph.op(value) {
                        Opcode::Projection
                            if graph.payload(value) == 2
                                && prefix.contains(&graph.input(value, 0)) =>
                        {
                            observed.insert(graph.input(value, 0));
                        }
                        Opcode::Add
                        | Opcode::Sub
                        | Opcode::Mul
                        | Opcode::BitAnd
                        | Opcode::BitOr
                        | Opcode::BitXor
                        | Opcode::Negate
                        | Opcode::BitNot
                        | Opcode::ShiftLeft
                        | Opcode::ShiftRightLogical
                        | Opcode::ShiftRightArithmetic
                        | Opcode::Compare
                        | Opcode::IntCast
                        | Opcode::ByteSwap
                        | Opcode::BitReverse => {
                            stack.extend_from_slice(graph.inputs(value));
                        }
                        _ => {}
                    }
                }
                if graph.op(*next) == Opcode::RequireDefined && graph.input(*next, 0) == effect {
                    control = graph.find_projection(*next, 0)?;
                    effect = graph.find_projection(*next, 1)?;
                } else {
                    return Some(observed);
                }
            }
            _ => return None,
        }
    }
    None
}
