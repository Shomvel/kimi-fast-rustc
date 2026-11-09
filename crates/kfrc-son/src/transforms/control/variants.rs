//! Preserve logical enum decisions after SSA promotion exposes their values.
//!
//! This pass asks only semantic discriminants and dominance. It never infers
//! enum identity from a pointer alias, layout, or the name of a source local.
use crate::analysis::cfg::ControlFlow;
use crate::analysis::dominators::Dominators;
use crate::analysis::uses::{UseKind, Uses};
use crate::{ComparePredicate, FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};
use kfrc_body::{TyId, TypeOracle, TypeShape};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VariantNormalizeStats {
    pub branches_normalized: usize,
    pub witnesses_added: usize,
    pub nodes_indexed: usize,
    pub use_edges_visited: usize,
    pub dirty_nodes: Vec<NodeId>,
}

/// Map source discriminant bits to exactly one logical variant. Signed
/// negative discriminants are normalized to their source integer width.
pub(crate) fn case_variant(
    graph: &FunctionGraph,
    enumeration: TyId,
    discriminant: TyId,
    bits: u128,
) -> Option<u32> {
    let count = graph.oracle.semantic().variant_count(enumeration)?;
    let TypeShape::Int { width, .. } = graph.oracle.shape(discriminant) else {
        return None;
    };
    let mask = u128::MAX >> (128 - u32::from(width.bits(64)));
    let mut matching = (0..count).filter(|&variant| {
        graph
            .oracle
            .semantic()
            .discriminant(enumeration, variant as u32)
            .is_some_and(|value| value & mask == bits & mask)
    });
    let first = matching.next()?;
    matching.next().is_none().then_some(first as u32)
}

pub fn normalize(graph: &mut FunctionGraph) -> VariantNormalizeStats {
    let mut stats = VariantNormalizeStats::default();
    if !matches!(graph.level, GraphLevel::Rust | GraphLevel::Specialized) {
        return stats;
    }
    graph.resolve_edges();
    let uses = Uses::new(graph);
    let nodes: Vec<_> = graph.ids().collect();
    stats.nodes_indexed += nodes.len();
    for branch in nodes
        .iter()
        .copied()
        .filter(|&node| graph.op(node) == Opcode::If)
        .collect::<Vec<_>>()
    {
        let condition = graph.input(branch, 0);
        if graph.op(condition) != Opcode::Compare {
            continue;
        }
        let predicate = ComparePredicate::from_payload(graph.payload(condition));
        if !matches!(
            predicate,
            ComparePredicate::Equal | ComparePredicate::NotEqual
        ) {
            continue;
        }
        let left = graph.input(condition, 0);
        let right = graph.input(condition, 1);
        let (query, constant) =
            if graph.op(left) == Opcode::Discriminant && graph.op(right) == Opcode::Const {
                (left, right)
            } else if graph.op(right) == Opcode::Discriminant && graph.op(left) == Opcode::Const {
                (right, left)
            } else {
                continue;
            };
        let source = graph.input(query, 0);
        let Some((source_ty, discriminant_ty)) =
            graph.rust_type(source).zip(graph.rust_type(query))
        else {
            continue;
        };
        let Some(variant) = case_variant(
            graph,
            source_ty,
            discriminant_ty,
            graph.constant_value(constant),
        ) else {
            continue;
        };
        let payload = graph.variant_switches.len() as u32;
        graph.variant_switches.push(vec![variant]);
        graph.set_opcode_payload(branch, Opcode::VariantSwitch, payload);
        graph.set_input(branch, 0, source);
        if predicate == ComparePredicate::NotEqual {
            for usage in uses.of(branch) {
                // Verified control tuple users are projections. Swapping their
                // case indices preserves all existing predecessor slots.
                graph.set_payload(usage.user, 1 - graph.payload(usage.user));
            }
        }
        stats.branches_normalized += 1;
        stats.dirty_nodes.push(branch);
    }
    // Snapshot users before attaching witnesses, so a new anchored use
    // cannot contaminate another field's proof search.
    let cfg = ControlFlow::new(graph);
    let dominators = Dominators::new(&cfg);
    let uses = Uses::new(graph);
    let proofs = variant_proofs(graph);
    stats.nodes_indexed += graph.ids().count() * 3;
    let mut candidates = Vec::new();
    for field in nodes
        .into_iter()
        .filter(|&node| graph.op(node) == Opcode::VariantField)
    {
        let source = graph.input(field, 0);
        let (variant, _) = graph.variant_fields[graph.payload(field) as usize];
        let Some(possible) = proofs.get(&(source, variant)) else {
            continue;
        };
        let Some(controls) = execution_uses(graph, &uses, field, &mut stats.use_edges_visited)
        else {
            continue;
        };
        if controls.is_empty() {
            continue;
        }
        if let Some(&proof) = possible.iter().find(|&&proof| {
            controls
                .iter()
                .all(|&control| dominators.dominates(&cfg, proof, control))
        }) {
            candidates.push((field, proof));
        }
    }
    let old_len = graph.len();
    stats.witnesses_added = insert_witnesses(graph, &cfg, &dominators, &proofs, &candidates);
    stats
        .dirty_nodes
        .extend(candidates.iter().map(|&(field, _)| field));
    stats
        .dirty_nodes
        .extend((old_len..graph.len()).map(|index| NodeId(index as u32)));
    stats.dirty_nodes.sort_unstable();
    stats.dirty_nodes.dedup();
    stats
}

/// Builder-provided source execution anchors use exactly the same proof
/// matching as graph normalization; a downcast alone is never a proof.
pub(crate) fn witness_candidates(
    graph: &mut FunctionGraph,
    candidates: &[(NodeId, NodeId)],
) -> usize {
    if candidates.is_empty() {
        return 0;
    }
    graph.resolve_edges();
    let cfg = ControlFlow::new(graph);
    let dominators = Dominators::new(&cfg);
    let proofs = variant_proofs(graph);
    insert_witnesses(graph, &cfg, &dominators, &proofs, candidates)
}

type Proofs = HashMap<(NodeId, u32), Vec<NodeId>>;
fn variant_proofs(graph: &FunctionGraph) -> Proofs {
    let mut proofs = HashMap::new();
    for projection in graph.ids() {
        if graph.op(projection) != Opcode::Projection
            || graph.value_type(projection) != &ValueType::Control
        {
            continue;
        }
        let branch = graph.input(projection, 0);
        if graph.op(branch) != Opcode::VariantSwitch {
            continue;
        }
        let source = graph.input(branch, 0);
        let cases = &graph.variant_switches[graph.payload(branch) as usize];
        let index = graph.payload(projection) as usize;
        let variant = if let Some(&variant) = cases.get(index) {
            Some(variant)
        } else if index == cases.len() {
            let count = graph
                .rust_type(source)
                .and_then(|ty| graph.oracle.semantic().variant_count(ty))
                .unwrap_or(0);
            let mut remaining = (0..count as u32).filter(|variant| !cases.contains(variant));
            let first = remaining.next();
            if remaining.next().is_none() {
                first
            } else {
                None
            }
        } else {
            None
        };
        if let Some(variant) = variant {
            proofs
                .entry((source, variant))
                .or_insert_with(Vec::new)
                .push(projection);
        }
    }
    proofs
}

fn insert_witnesses(
    graph: &mut FunctionGraph,
    cfg: &ControlFlow,
    dominators: &Dominators,
    proofs: &Proofs,
    candidates: &[(NodeId, NodeId)],
) -> usize {
    let mut cache = HashMap::new();
    for &(field, anchor) in candidates {
        let field = graph.resolve(field);
        let anchor = graph.resolve(anchor);
        if graph.is_dead(field) || graph.op(field) != Opcode::VariantField {
            continue;
        }
        let source = graph.input(field, 0);
        let (variant, _) = graph.variant_fields[graph.payload(field) as usize];
        let Some(proof) = proofs.get(&(source, variant)).and_then(|proofs| {
            proofs
                .iter()
                .copied()
                .find(|proof| dominators.dominates(cfg, *proof, anchor))
        }) else {
            continue;
        };
        if graph.inputs(field).len() != 1 {
            continue;
        }
        let witness = *cache
            .entry((source, variant, anchor, proof))
            .or_insert_with(|| {
                graph.add_node(
                    Opcode::VariantWitness,
                    anchor,
                    &[source, proof],
                    variant,
                    ValueType::Witness,
                )
            });
        // Preserve the enum's original value identity; the second operand is
        // a compiler proof, never another Rust value or a runtime argument.
        graph.set_inputs(field, &[source, witness]);
    }
    cache.len()
}

fn execution_uses(
    graph: &FunctionGraph,
    uses: &Uses,
    value: NodeId,
    visits: &mut usize,
) -> Option<Vec<NodeId>> {
    let mut pending = vec![value];
    let mut seen = HashSet::new();
    let mut controls = Vec::new();
    while let Some(node) = pending.pop() {
        if !seen.insert(node) {
            continue;
        }
        // Bound shared pure-cone exploration. Unknown cases remain unchanged.
        if seen.len() > 512 {
            return None;
        }
        for usage in uses.of(node) {
            *visits += 1;
            match usage.kind {
                UseKind::Control => return None,
                UseKind::Operand(slot) if graph.op(usage.user) == Opcode::Phi => {
                    if slot == 0 {
                        return None;
                    }
                    controls.push(*graph.inputs(graph.input(usage.user, 0)).get(slot - 1)?);
                }
                UseKind::Operand(_) if !graph.control(usage.user).is_none() => {
                    controls.push(graph.control(usage.user))
                }
                UseKind::Operand(_) if !graph.op(usage.user).is_fixed() => pending.push(usage.user),
                _ => return None,
            }
        }
    }
    Some(controls)
}
