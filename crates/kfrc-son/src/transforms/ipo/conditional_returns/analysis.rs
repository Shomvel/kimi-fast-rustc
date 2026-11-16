//! Follow one actual incoming edge through Region/Phi shells. Only the
//! summarized path must be empty of observations; the other path stays a call.
use super::{MAX_FIELD_DEPTH, MAX_PATH_STEPS, Selector, Summary};
use crate::{ComparePredicate, FunctionGraph, NodeId, Opcode, ValueType};
use kfrc_body::{TypeOracle, TypeShape};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn summarize(graph: &FunctionGraph) -> Option<Summary> {
    // Definitions come from the verified S pipeline. Keep this read-only check
    // at the same boundary as other IPO sources, before reading payloads.
    crate::verify::check_pipeline_step(graph).ok()?;
    let ValueType::Tuple(parameters) = graph.value_type(graph.start) else {
        return None;
    };
    if parameters.len() != graph.abi.args.len() + 2
        || parameters.get(0) != Some(&ValueType::Control)
        || parameters.get(1) != Some(&ValueType::Effect)
    {
        return None;
    }
    let index = Index::new(graph)?;
    let entry = graph.find_projection(graph.start, 0)?;
    let mut prefix = Path::new();
    let mut control = entry;
    let guard = loop {
        prefix.step(control)?;
        let next = index.next(control)?;
        match graph.op(next) {
            Opcode::If => break next,
            Opcode::Jump => control = prefix.enter_region(graph, &index, next)?,
            _ => return None,
        }
    };
    let (lhs, rhs, predicate, negated) = condition(graph, graph.input(guard, 0))?;
    for branch in [0, 1] {
        let mut path = prefix.clone();
        let Some(mut control) = graph.find_projection(guard, branch) else {
            continue;
        };
        let result = loop {
            if path.step(control).is_none() {
                break None;
            }
            let Some(next) = index.next(control) else {
                break None;
            };
            match graph.op(next) {
                Opcode::Jump => {
                    let Some(region) = path.enter_region(graph, &index, next) else {
                        break None;
                    };
                    control = region;
                }
                Opcode::Return => {
                    if graph.inputs(next).len() != 2 {
                        break None;
                    }
                    let effect = path.selected(graph, graph.input(next, 0), &mut 64);
                    let value = path.selected(graph, graph.input(next, 1), &mut 64);
                    let (Some(effect), Some(value)) = (effect, value) else {
                        break None;
                    };
                    if graph.op(effect) != Opcode::Projection
                        || graph.input(effect, 0) != graph.start
                        || graph.payload(effect) != 1
                        || graph.op(value) != Opcode::Const
                        || !graph
                            .rust_type(value)
                            .is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Bool))
                        || graph.constant_value(value) > 1
                    {
                        break None;
                    }
                    break Some((
                        graph.value_type(value).clone(),
                        graph.constant_value(value) != 0,
                    ));
                }
                // Even an otherwise unused Assume, RequireDefined, anchored
                // division, memory read, or call prevents this proof.
                _ => break None,
            }
        };
        if let Some((boolean, result)) = result {
            let invert = negated ^ (branch == 1);
            return Some(Summary {
                abi: graph.abi.clone(),
                oracle: graph.oracle.clone(),
                parameters: graph.value_type(graph.start).clone(),
                boolean,
                lhs,
                rhs,
                predicate: if invert {
                    inverse(predicate)
                } else {
                    predicate
                },
                result,
                nodes_visited: graph.live_count(),
            });
        }
    }
    None
}

fn inverse(predicate: ComparePredicate) -> ComparePredicate {
    match predicate {
        ComparePredicate::Equal => ComparePredicate::NotEqual,
        ComparePredicate::NotEqual => ComparePredicate::Equal,
        _ => unreachable!("only equality relations are summarized"),
    }
}

fn condition(
    graph: &FunctionGraph,
    mut node: NodeId,
) -> Option<(Selector, Selector, ComparePredicate, bool)> {
    node = graph.resolve(node);
    let negated = graph.op(node) == Opcode::BitNot;
    if negated {
        if !graph.control(node).is_none()
            || !graph
                .rust_type(node)
                .is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Bool))
        {
            return None;
        }
        node = graph.resolve(graph.input(node, 0));
    }
    if graph.op(node) != Opcode::Compare
        || !graph.control(node).is_none()
        || graph.inputs(node).len() != 2
        || !graph
            .rust_type(node)
            .is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Bool))
    {
        return None;
    }
    let predicate = match graph.payload(node) {
        n if n == ComparePredicate::Equal as u32 => ComparePredicate::Equal,
        n if n == ComparePredicate::NotEqual as u32 => ComparePredicate::NotEqual,
        _ => return None,
    };
    let a = graph.resolve(graph.input(node, 0));
    let b = graph.resolve(graph.input(node, 1));
    if graph.value_type(a) != graph.value_type(b)
        || !graph
            .rust_type(a)
            .is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Int { .. }))
    {
        return None;
    }
    Some((selector(graph, a)?, selector(graph, b)?, predicate, negated))
}

fn selector(graph: &FunctionGraph, mut node: NodeId) -> Option<Selector> {
    let mut fields = Vec::new();
    loop {
        node = graph.resolve(node);
        if !graph.control(node).is_none() {
            return None;
        }
        if graph.op(node) != Opcode::Field {
            break;
        }
        if fields.len() == MAX_FIELD_DEPTH || graph.inputs(node).len() != 1 {
            return None;
        }
        let source = graph.resolve(graph.input(node, 0));
        if graph
            .oracle
            .semantic()
            .field_type(graph.rust_type(source)?, None, graph.payload(node))
            != graph.rust_type(node)
        {
            return None;
        }
        fields.push(graph.payload(node));
        node = source;
    }
    if graph.op(node) != Opcode::Projection || graph.input(node, 0) != graph.start {
        return None;
    }
    let parameter = graph.payload(node).checked_sub(2)? as usize;
    fields.reverse();
    Some(Selector { parameter, fields })
}

struct Index {
    /// Include all anchored nodes, not just effect-producing ones. Thus a
    /// second, unused trapping arithmetic node makes `next` reject the path.
    anchored: Vec<Vec<NodeId>>,
    region_of_jump: Vec<Option<(NodeId, usize)>>,
    phis: Vec<Vec<NodeId>>,
}

impl Index {
    fn new(graph: &FunctionGraph) -> Option<Self> {
        let mut index = Self {
            anchored: vec![Vec::new(); graph.len()],
            region_of_jump: vec![None; graph.len()],
            phis: vec![Vec::new(); graph.len()],
        };
        for node in graph.ids() {
            match graph.op(node) {
                Opcode::Phi => index.phis[graph.input(node, 0).index()].push(node),
                Opcode::Region => {
                    for (slot, &jump) in graph.inputs(node).iter().enumerate() {
                        if index.region_of_jump[jump.index()]
                            .replace((node, slot))
                            .is_some()
                        {
                            return None;
                        }
                    }
                }
                _ => {}
            }
            let control = graph.control(node);
            if !control.is_none() && graph.op(node) != Opcode::Phi {
                index.anchored[control.index()].push(node);
            }
        }
        Some(index)
    }

    fn next(&self, control: NodeId) -> Option<NodeId> {
        match self.anchored.get(control.index())?.as_slice() {
            &[next] => Some(next),
            _ => None,
        }
    }
}

#[derive(Clone)]
struct Path {
    incoming: BTreeMap<NodeId, usize>,
    seen: BTreeSet<NodeId>,
    fuel: usize,
}

impl Path {
    fn new() -> Self {
        Self {
            incoming: BTreeMap::new(),
            seen: BTreeSet::new(),
            fuel: MAX_PATH_STEPS,
        }
    }

    fn step(&mut self, control: NodeId) -> Option<()> {
        self.fuel = self.fuel.checked_sub(1)?;
        self.seen.insert(control).then_some(())
    }

    fn enter_region(
        &mut self,
        graph: &FunctionGraph,
        index: &Index,
        jump: NodeId,
    ) -> Option<NodeId> {
        let (region, slot) = index.region_of_jump[jump.index()]?;
        if self.seen.contains(&region) || !index.anchored[jump.index()].is_empty() {
            return None;
        }
        self.incoming.insert(region, slot);
        // Regions evaluate every selected Phi input, including unused Phis.
        // Inspect all of them rather than just the eventual return value.
        let mut fuel = 128;
        for &phi in &index.phis[region.index()] {
            self.safe_value(graph, graph.input(phi, slot + 1), &mut fuel)?;
        }
        Some(region)
    }

    fn selected(&self, graph: &FunctionGraph, node: NodeId, fuel: &mut usize) -> Option<NodeId> {
        *fuel = fuel.checked_sub(1)?;
        let node = graph.resolve(node);
        if graph.op(node) == Opcode::Phi {
            let region = graph.input(node, 0);
            let slot = *self.incoming.get(&region)?;
            self.selected(graph, graph.input(node, slot + 1), fuel)
        } else {
            Some(node)
        }
    }

    fn safe_value(&self, graph: &FunctionGraph, node: NodeId, fuel: &mut usize) -> Option<()> {
        let node = self.selected(graph, node, fuel)?;
        if !graph.control(node).is_none() {
            return None;
        }
        match graph.op(node) {
            Opcode::Projection if graph.input(node, 0) == graph.start => Some(()),
            Opcode::Const => Some(()),
            Opcode::Field | Opcode::BitNot => self.safe_value(graph, graph.input(node, 0), fuel),
            Opcode::Compare
                if matches!(
                    graph.payload(node),
                    n if n == ComparePredicate::Equal as u32 || n == ComparePredicate::NotEqual as u32
                ) =>
            {
                self.safe_value(graph, graph.input(node, 0), fuel)?;
                self.safe_value(graph, graph.input(node, 1), fuel)
            }
            _ => None,
        }
    }
}
