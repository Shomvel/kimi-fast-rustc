//! Demand-driven R→S specialization. Control questions are answered before any
//! unreachable node or type is copied. The prepared definition is immutable and
//! shared by all instances; optimization and CFG indexing happen once.
mod clone;
mod constant;
mod dependencies;
mod inhabited;
mod prepare_control;

use crate::analysis::cfg::ControlFlow;
use crate::{FunctionGraph, GraphLevel, GraphState, Hole, NodeId, Opcode, ValueType};
use kfrc_body::{EnumLayout, FunctionAbi, RecordedTypes, SizeAlign, TyId, TypeOracle, TypeShape};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HoleAnswer {
    SizeAlign(SizeAlign),
    NeedsDrop(bool),
    Resolved {
        target: crate::CallTarget,
        identity: String,
    },
    VariantEncoding(EnumLayout),
}

/// TyIds and questions belong to the definition. Implementations instantiate
/// only the requested type/question; they must not eagerly enumerate all holes.
pub trait SpecializationOracle {
    fn instantiate_type(&mut self, ty: TyId, into: &mut RecordedTypes) -> Result<TyId, String>;
    fn answer(&mut self, hole: &Hole) -> Result<HoleAnswer, String>;
    fn function_abi(&mut self) -> Result<FunctionAbi, String>;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpecializationStats {
    pub control_nodes_visited: usize,
    pub value_nodes_visited: usize,
    pub nodes_copied: usize,
    pub branches_pruned: usize,
    pub holes_answered: usize,
    pub types_instantiated: usize,
    pub impossible_boundaries: usize,
}

#[derive(Clone, Debug)]
pub struct Specialization {
    pub graph: FunctionGraph,
    /// First demand order, suitable for auditing a specialization identity.
    /// Consumers must include semantic type observations and callee identities
    /// before reusing typed code; equal sizes alone are never a cache key.
    pub answers: Vec<(u32, HoleAnswer)>,
    pub stats: SpecializationStats,
    /// Node identities are stable until the caller's seam pipeline compacts.
    pub dirty_nodes: Vec<NodeId>,
    /// Live logical storage candidates whose type or users changed.
    pub local_slots: Vec<NodeId>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreparationStats {
    pub optimization_enabled: bool,
    pub nodes_before: usize,
    pub nodes_after: usize,
    pub slots_promoted: usize,
    pub loads_removed: usize,
    pub stores_removed: usize,
    pub branches_normalized: usize,
    pub witnesses_added: usize,
    pub promotion_node_visits: usize,
    pub promotion_edge_visits: usize,
    pub normalization_nodes_indexed: usize,
    pub normalization_use_edges_visited: usize,
    /// Shared CFG sweeps run only during definition preparation.
    pub control_simplification_runs: usize,
    pub branches_folded: usize,
    pub predecessor_edges_removed: usize,
    pub control_nodes_removed: usize,
    /// The preparation change snapshot, not an estimate of the shared CFG
    /// implementation's internal traversal count.
    pub control_snapshot_nodes: usize,
    pub control_snapshot_edges: usize,
    pub control_changed_use_edges: usize,
    pub control_rule_nodes_indexed: usize,
    pub control_rule_nodes_visited: usize,
    pub control_constants_folded: usize,
    pub control_identities_simplified: usize,
    pub control_fields_folded: usize,
    pub control_variants_folded: usize,
}

#[derive(Clone, Debug)]
pub struct RustDefinition {
    graph: FunctionGraph,
    cfg: ControlFlow,
    anchored: HashMap<NodeId, Vec<NodeId>>,
    parameter_dependent: HashSet<NodeId>,
    preparation: PreparationStats,
}

impl RustDefinition {
    pub fn prepare(graph: FunctionGraph) -> Result<Self, String> {
        Self::prepare_with_optimization(graph, true)
    }

    pub fn prepare_with_optimization(
        mut graph: FunctionGraph,
        optimize: bool,
    ) -> Result<Self, String> {
        if graph.level != GraphLevel::Rust {
            return Err("definition preparation requires an R graph".into());
        }
        crate::verify_graph(&mut graph).map_err(|error| error.to_string())?;
        // This is definition work, performed once before any concrete answers.
        let mut preparation = PreparationStats {
            optimization_enabled: optimize,
            nodes_before: graph.ids().count(),
            ..PreparationStats::default()
        };
        if optimize {
            let promoted = crate::transforms::memory::promotion::run(&mut graph, None);
            preparation.slots_promoted = promoted.slots_promoted;
            preparation.loads_removed = promoted.loads_removed;
            preparation.stores_removed = promoted.stores_removed;
            preparation.promotion_node_visits = promoted.node_visits;
            preparation.promotion_edge_visits = promoted.edge_visits;
            // Canonicalize repeated pure projections before deriving facts:
            // proof matching intentionally uses exact SSA identity.
            crate::optimize_graph(&mut graph);
            let normalized = crate::transforms::control::variants::normalize(&mut graph);
            preparation.branches_normalized = normalized.branches_normalized;
            preparation.witnesses_added = normalized.witnesses_added;
            preparation.normalization_nodes_indexed = normalized.nodes_indexed;
            preparation.normalization_use_edges_visited = normalized.use_edges_visited;
            prepare_control::run(&mut graph, &mut preparation)?;
            graph.compact();
        }
        preparation.nodes_after = graph.ids().count();
        crate::verify_graph(&mut graph).map_err(|error| error.to_string())?;
        let cfg = ControlFlow::new(&graph);
        let mut anchored: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for node in graph
            .ids()
            .filter(|node| graph.op(*node).is_fixed() && !cfg.contains(*node))
        {
            anchored.entry(graph.control(node)).or_default().push(node);
        }
        let parameter_dependent = dependencies::parameter_dependent_nodes(&graph);
        Ok(Self {
            graph,
            cfg,
            anchored,
            parameter_dependent,
            preparation,
        })
    }

    pub fn preparation_stats(&self) -> &PreparationStats {
        &self.preparation
    }

    pub fn graph(&self) -> &FunctionGraph {
        &self.graph
    }

    pub fn specialize(
        &self,
        symbol: impl Into<String>,
        oracle: &mut impl SpecializationOracle,
    ) -> Result<Specialization, String> {
        let abi = oracle.function_abi()?;
        let signature = self
            .graph
            .rust_signature
            .as_ref()
            .ok_or("R signature is absent")?;
        if abi.args.len() != signature.args.len() {
            return Err("specialization requires a source-to-ABI argument adapter".into());
        }
        let mut state = State {
            source: &self.graph,
            parameter_dependent: &self.parameter_dependent,
            dirty: HashSet::new(),
            oracle,
            target: FunctionGraph::new(symbol, abi, RecordedTypes::new()),
            answers: Vec::new(),
            answer_index: HashMap::new(),
            constants: HashMap::new(),
            evaluating: HashSet::new(),
            reachable: HashSet::new(),
            selected: HashMap::new(),
            impossible: HashSet::new(),
            nodes: HashMap::new(),
            types: HashMap::new(),
            stats: SpecializationStats::default(),
        };
        // Discover only executable control. A folded branch does not enqueue
        // its rejected arm, so its callees, layouts and drop questions stay cold.
        let mut queue = VecDeque::from([self.graph.start]);
        let mut visited = Vec::new();
        while let Some(node) = queue.pop_front() {
            if !state.reachable.insert(node) {
                continue;
            }
            state.stats.control_nodes_visited += 1;
            visited.push(node);
            let boundary_types = match self.graph.op(node) {
                Opcode::Start => signature.args.clone(),
                Opcode::TraitCall => vec![
                    self.graph.trait_calls[self.graph.payload(node) as usize]
                        .signature
                        .ret,
                ],
                _ => Vec::new(),
            };
            let mut impossible = false;
            for ty in boundary_types {
                let ty = state.ty(ty)?;
                impossible |= inhabited::impossible(&state.target.oracle, ty);
            }
            if impossible {
                state.impossible.insert(node);
                state.stats.impossible_boundaries += 1;
                // The source signature forbids a continuation. Keep the call
                // itself (if any), but never manufacture its absent result or
                // clone nodes that consume an impossible incoming parameter.
                continue;
            }
            queue.extend(self.anchored.get(&node).into_iter().flatten().copied());
            let selection = match self.graph.op(node) {
                Opcode::If => state
                    .constant(self.graph.input(node, 0))?
                    .map(|value| u32::from(value == 0)),
                Opcode::VariantSwitch => {
                    let value = self.graph.input(node, 0);
                    if self.graph.op(value) == Opcode::MakeVariant {
                        let variant = self.graph.payload(value);
                        let cases = &self.graph.variant_switches[self.graph.payload(node) as usize];
                        Some(
                            cases
                                .iter()
                                .position(|case| *case == variant)
                                .unwrap_or(cases.len()) as u32,
                        )
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(selection) = selection {
                state.selected.insert(node, selection);
                state.stats.branches_pruned += 1;
            }
            for &next in self.cfg.successors(node) {
                if let Some(selection) = selection {
                    if self.graph.op(next) == Opcode::Projection
                        && self.graph.payload(next) != selection
                    {
                        continue;
                    }
                }
                queue.push_back(next);
            }
        }
        // Keep original creation order for deterministic output, but allocate
        // dependencies recursively. Neither rejected data nor its type is read.
        for node in visited {
            if state.reachable.contains(&node) && !state.selected.contains_key(&node) {
                let copied = state.copy(node)?;
                if state.impossible.contains(&node) {
                    let after = state.target.add_node(
                        Opcode::Projection,
                        NodeId::NONE,
                        &[copied],
                        0,
                        ValueType::Control,
                    );
                    let unreachable = state.target.add_node(
                        Opcode::Unreachable,
                        after,
                        &[],
                        0,
                        ValueType::Control,
                    );
                    state.dirty.extend([copied, after, unreachable]);
                }
            }
        }
        state.target.start = state.copy(self.graph.start)?;
        state.target.state = GraphState::Sealed;
        crate::verify_graph(&mut state.target).map_err(|error| error.to_string())?;
        // Include direct users once; the existing seam worklist propagates
        // successful replacements transitively. Never rerun every R rule on S.
        let uses = crate::analysis::uses::Uses::new(&state.target);
        let roots: Vec<_> = state.dirty.iter().copied().collect();
        for root in roots {
            state
                .dirty
                .extend(uses.of(root).iter().map(|usage| usage.user));
        }
        let mut dirty_nodes: Vec<_> = state.dirty.into_iter().collect();
        dirty_nodes.sort_unstable();
        let local_slots = dirty_nodes
            .iter()
            .copied()
            .filter(|node| state.target.op(*node) == Opcode::LocalSlot)
            .collect();
        Ok(Specialization {
            graph: state.target,
            answers: state.answers,
            stats: state.stats,
            dirty_nodes,
            local_slots,
        })
    }
}

struct State<'a, O> {
    source: &'a FunctionGraph,
    parameter_dependent: &'a HashSet<NodeId>,
    dirty: HashSet<NodeId>,
    oracle: &'a mut O,
    target: FunctionGraph,
    answers: Vec<(u32, HoleAnswer)>,
    answer_index: HashMap<u32, usize>,
    constants: HashMap<NodeId, Option<u128>>,
    evaluating: HashSet<NodeId>,
    reachable: HashSet<NodeId>,
    selected: HashMap<NodeId, u32>,
    impossible: HashSet<NodeId>,
    nodes: HashMap<NodeId, NodeId>,
    types: HashMap<TyId, TyId>,
    stats: SpecializationStats,
}

impl<O: SpecializationOracle> State<'_, O> {
    fn answer(&mut self, index: u32) -> Result<HoleAnswer, String> {
        if let Some(&index) = self.answer_index.get(&index) {
            return Ok(self.answers[index].1.clone());
        }
        let question = self
            .source
            .holes
            .get(index as usize)
            .ok_or("hole index is invalid")?;
        let answer = self.oracle.answer(question)?;
        let valid = matches!(
            (question, &answer),
            (Hole::SizeAlign(_), HoleAnswer::SizeAlign(_))
                | (Hole::NeedsDrop(_), HoleAnswer::NeedsDrop(_))
                | (Hole::Resolve { .. }, HoleAnswer::Resolved { .. })
                | (Hole::VariantEncoding(_), HoleAnswer::VariantEncoding(_))
        );
        if !valid {
            return Err("specialization oracle returned the wrong answer kind".into());
        }
        self.answer_index.insert(index, self.answers.len());
        self.answers.push((index, answer.clone()));
        self.stats.holes_answered += 1;
        Ok(answer)
    }
    fn ty(&mut self, ty: TyId) -> Result<TyId, String> {
        if let Some(&mapped) = self.types.get(&ty) {
            return Ok(mapped);
        }
        let mapped = if let TypeShape::PlaceAddress { pointee } = self.source.oracle.shape(ty) {
            // Internal storage addresses have no rustc Ty identity. Instantiate
            // their semantic owner, then rebuild the same internal address.
            let pointee = self.ty(*pointee)?;
            self.target
                .oracle
                .intern(TypeShape::PlaceAddress { pointee }, None)
        } else {
            self.oracle.instantiate_type(ty, &mut self.target.oracle)?
        };
        self.types.insert(ty, mapped);
        self.stats.types_instantiated += 1;
        Ok(mapped)
    }
    fn value_type(&mut self, ty: &ValueType) -> Result<ValueType, String> {
        Ok(match ty {
            ValueType::Rust(ty) => ValueType::Rust(self.ty(*ty)?),
            ValueType::Tuple(fields) => ValueType::Tuple(
                fields
                    .iter()
                    .map(|ty| self.value_type(ty))
                    .collect::<Result<_, _>>()?,
            ),
            ValueType::Control => ValueType::Control,
            ValueType::Effect => ValueType::Effect,
            ValueType::Witness => ValueType::Witness,
            ValueType::LayoutValue(_) => return Err("R contains a physical layout value".into()),
        })
    }
}
