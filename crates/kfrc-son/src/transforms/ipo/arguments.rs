//! Call-site facts refine literal scalar values. One account bounds the full
//! preparation tree; pointers and unselected fields retain their identities.
mod cleanup;
mod dump;
mod facts;
mod reapply;

use super::cost::{Costs, PreparationDecision, PreparationStats};
use super::{GraphProvider, InlineBudget, InlineError, InlineSkip, InlineStats};
use crate::{FunctionGraph, NodeId, Opcode};
use kfrc_body::fx_hash::HashMap;
use kfrc_body::{TypeOracle, TypeShape};
use std::rc::Rc;

const MAX_SOURCE_NODES: usize = 512;
const MAX_FACTS: usize = 16;
const MAX_FIELD_DEPTH: usize = 2;
pub(super) const MAX_DEPTH: usize = 6;
/// One account covers nonfree splices across all preparation frames. Free
/// helpers still consume the same nonrefundable node-clone account.
const MAX_HELPERS: usize = 8;
const MAX_DECISIONS: usize = 256;
const PRIORITY_VISITS_PER_CLONE_NODE: usize = 4;
const REAPPLY_VISITS_PER_CLONE_NODE: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Fact {
    parameter: u32,
    fields: Vec<u32>,
    shape: TypeShape,
    bits: u128,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    symbol: String,
    facts: Vec<Fact>,
    remaining_depth: usize,
}

struct Active {
    symbol: String,
    refinements: reapply::Refinements,
}

#[derive(Clone, Copy)]
enum Cached {
    Prepared(usize),
    Unchanged,
}

pub(super) struct Prepared {
    pub index: usize,
    pub source: Rc<FunctionGraph>,
    pub cached: bool,
}

/// Scratch emits no object. The final outer plan retains every linkage and
/// data-identity check; source visibility still belongs to the real provider.
struct Preparation<'a, P: GraphProvider + ?Sized>(&'a P);
impl<P: GraphProvider + ?Sized> GraphProvider for Preparation<'_, P> {
    fn graph(&self, symbol: &str) -> Option<&FunctionGraph> {
        self.0.preparation_graph(symbol)
    }

    fn always_inline(&self, symbol: &str) -> bool {
        self.0.always_inline(symbol)
    }
}

pub(super) struct Arguments {
    cache: HashMap<Key, Cached>,
    sources: Vec<Rc<FunctionGraph>>,
    active: Vec<Active>,
    remaining_nodes: usize,
    initial_nodes: usize,
    remaining_priority_visits: usize,
    remaining_reapply_visits: usize,
    remaining_helpers: usize,
    nodes_cloned: usize,
    cache_hits: usize,
    stats: PreparationStats,
    dump: dump::Dump,
}

impl Arguments {
    pub(super) fn supports_source(source: &FunctionGraph) -> bool {
        source.len() <= MAX_SOURCE_NODES
    }

    pub(super) fn new(max_nodes: usize) -> Self {
        let priority_visit_budget = max_nodes.saturating_mul(PRIORITY_VISITS_PER_CLONE_NODE);
        let fact_reapplication_visit_budget =
            max_nodes.saturating_mul(REAPPLY_VISITS_PER_CLONE_NODE);
        Self {
            cache: HashMap::default(),
            sources: Vec::new(),
            active: Vec::new(),
            remaining_nodes: max_nodes,
            initial_nodes: max_nodes,
            // Analysis gets one separate, nonrefundable visit account for the
            // whole tree. This grants no extra node-clone or splice fuel.
            remaining_priority_visits: priority_visit_budget,
            remaining_reapply_visits: fact_reapplication_visit_budget,
            remaining_helpers: MAX_HELPERS,
            nodes_cloned: 0,
            cache_hits: 0,
            stats: PreparationStats {
                priority_visit_budget,
                fact_reapplication_visit_budget,
                ..PreparationStats::default()
            },
            dump: dump::Dump::new(),
        }
    }

    pub(super) fn remaining(&self) -> usize {
        self.remaining_nodes
    }
    pub(super) fn helpers_available(&self, free: bool) -> bool {
        free || self.remaining_helpers != 0
    }

    pub(super) fn priority_remaining(&self) -> usize {
        self.remaining_priority_visits
    }

    pub(super) fn priority_work(&mut self, visits: usize, bailed_out: bool) {
        self.remaining_priority_visits -= visits;
        self.stats.nodes_visited += visits;
        self.stats.priority_node_visits += visits;
        self.stats.priority_budget_bailouts += usize::from(bailed_out);
    }

    /// Use the same scalar/aggregate selection and poison-conditional facts as
    /// preparation itself. Priority only chooses work; it never replaces an
    /// argument or relaxes its call-boundary observation.
    pub(super) fn has_scalar_fact(
        graph: &FunctionGraph,
        call: NodeId,
        max_visits: usize,
    ) -> (bool, usize) {
        let mut facts = Vec::new();
        let mut fuel = max_visits.min(MAX_FACTS * (MAX_FIELD_DEPTH + 1));
        let original = fuel;
        for (parameter, &argument) in graph.inputs(call)[1..].iter().enumerate() {
            if fuel == 0 || !facts.is_empty() {
                break;
            }
            collect(
                graph,
                graph.resolve(argument),
                parameter as u32,
                &mut Vec::new(),
                &mut facts,
                &mut fuel,
            );
        }
        (!facts.is_empty(), original - fuel)
    }
    pub(super) fn limited(&mut self) {
        self.stats.budget_bailouts += 1;
    }

    /// Charge the second full graph copy made by the transactional scratch
    /// driver before it can mutate anything. Emission's caller copy is separate.
    pub(super) fn transaction(&mut self, nodes: usize) {
        self.remaining_nodes -= nodes;
        self.nodes_cloned += nodes;
        self.stats.transaction_nodes_cloned += nodes;
    }

    /// Preflight restricts plan growth to this same account. Boundary nodes
    /// consume fuel too, even though they are not source-node clones.
    pub(super) fn spliced(&mut self, allocated: usize, cloned: usize, free: bool, cached: bool) {
        self.remaining_nodes -= allocated;
        if free {
            self.stats.free_calls_inlined += 1;
        } else {
            self.remaining_helpers -= 1;
            self.stats.costed_calls_inlined += 1;
        }
        if cached {
            self.stats.cached_calls_inlined += 1;
            self.stats.cached_splice_nodes_allocated += allocated;
        }
        self.nodes_cloned += cloned;
        self.stats.splice_nodes_allocated += allocated;
        self.stats.splice_nodes_cloned += cloned;
    }

    pub(super) fn clean(&mut self, graph: &mut FunctionGraph) -> Result<bool, InlineError> {
        let result = cleanup::run(graph, |graph| self.reapply(graph))
            .map_err(InlineError::Specialization)?;
        self.stats.cleanup_runs += 1;
        self.stats.cleanup_rounds += result.rounds;
        self.stats.cleanup_nodes_indexed += result.nodes_indexed;
        self.stats.cleanup_node_visits += result.node_visits;
        self.stats.cleanup_nodes_added += result.nodes_added;
        self.stats.cleanup_branches_folded += result.branches_folded;
        self.stats.cleanup_observations_removed += result.observations_removed;
        self.stats.cleanup_conditions_folded += result.conditions_folded;
        if !result.complete {
            self.limited();
        }
        Ok(result.complete)
    }

    fn reapply(&mut self, graph: &mut FunctionGraph) -> usize {
        let Some(active) = self.active.last_mut() else {
            return 0;
        };
        // Facts belong to this preparation frame's parameters, never to a
        // nested source or the emitted caller that happens to share a session.
        if active.symbol != graph.symbol {
            return 0;
        }
        let work = active.refinements.run(
            graph,
            &mut self.remaining_reapply_visits,
            &mut self.remaining_nodes,
        );
        self.stats.nodes_visited += work.visits;
        self.stats.fact_reapplication_node_visits += work.visits;
        self.stats.fact_reapplication_fields_replaced += work.replaced;
        self.stats.fact_reapplication_nodes_allocated += work.allocated;
        self.stats.fact_reapplication_budget_bailouts += usize::from(work.limited);
        if work.limited {
            self.limited();
        }
        work.replaced
    }

    fn record(
        &mut self,
        owner: &str,
        symbol: &str,
        depth: usize,
        source_nodes: usize,
        specialized_nodes: Option<usize>,
        action: &'static str,
        rejection: Option<InlineSkip>,
    ) {
        self.stats.maximum_depth = self.stats.maximum_depth.max(depth);
        if self.stats.decisions.len() == MAX_DECISIONS {
            self.stats.decisions_dropped += 1;
            return;
        }
        self.stats.decisions.push(PreparationDecision {
            root: self
                .active
                .first()
                .map(|active| active.symbol.clone())
                .unwrap_or_else(|| symbol.to_owned()),
            owner: owner.to_owned(),
            symbol: symbol.to_owned(),
            depth,
            source_nodes,
            specialized_nodes,
            action,
            rejection,
        });
    }

    pub(super) fn record_driver(&mut self, owner: &str, stats: &mut InlineStats) {
        self.stats.calls_inlined += stats.calls_inlined;
        self.stats.nodes_visited += stats.nodes_visited;
        for decision in stats.decisions.drain(..) {
            if decision.rejection == Some(InlineSkip::Budget) {
                self.limited();
            }
            self.record(
                owner,
                &decision.symbol,
                decision.depth,
                decision.source_nodes,
                decision.specialized_nodes,
                "splice",
                decision.rejection,
            );
        }
    }

    pub(super) fn finish(&mut self, stats: &mut InlineStats) {
        debug_assert_eq!(
            self.initial_nodes - self.remaining_nodes,
            self.stats.source_nodes_cloned
                + self.stats.transaction_nodes_cloned
                + self.stats.splice_nodes_allocated
                + self.stats.fact_reapplication_nodes_allocated
        );
        debug_assert!(
            self.stats.fact_reapplication_node_visits <= self.stats.fact_reapplication_visit_budget
        );
        debug_assert_eq!(
            self.stats.calls_inlined,
            self.stats.costed_calls_inlined + self.stats.free_calls_inlined
        );
        stats.specializations_prepared = self.sources.len();
        stats.specialization_nodes_cloned = self.nodes_cloned;
        stats.specialization_cache_hits = self.cache_hits;
        self.stats.cache_hits = self.cache_hits;
        self.stats.clone_budget_used = self.initial_nodes - self.remaining_nodes;
        stats.preparation = std::mem::take(&mut self.stats);
    }

    /// Zero identifies the original; all prepared-source indices belong to this
    /// one session and therefore also to the shared cost/import cache namespace.
    pub(super) fn prepare(
        &mut self,
        caller: &FunctionGraph,
        call: NodeId,
        source: &FunctionGraph,
        provider: &(impl GraphProvider + ?Sized),
        costs: &mut Costs<'_>,
        depth: usize,
    ) -> Result<Option<Prepared>, InlineError> {
        if !Self::supports_source(source) {
            return Ok(None);
        }
        if depth >= MAX_DEPTH {
            self.limited();
            self.record(
                &caller.symbol,
                &source.symbol,
                depth,
                source.live_count(),
                None,
                "depth",
                Some(InlineSkip::Budget),
            );
            return Ok(None);
        }
        let mut facts = Vec::new();
        let mut fuel = MAX_FACTS * (MAX_FIELD_DEPTH + 1);
        let original_fuel = fuel;
        for (parameter, &argument) in caller.inputs(call)[1..].iter().enumerate() {
            if fuel == 0 || facts.len() == MAX_FACTS {
                break;
            }
            collect(
                caller,
                caller.resolve(argument),
                parameter as u32,
                &mut Vec::new(),
                &mut facts,
                &mut fuel,
            );
        }
        self.stats.fact_node_visits += original_fuel - fuel;
        if facts.is_empty() {
            return Ok(None);
        }
        let key = Key {
            symbol: source.symbol.clone(),
            facts,
            remaining_depth: MAX_DEPTH - depth,
        };
        if let Some(&cached) = self.cache.get(&key) {
            self.cache_hits += 1;
            let result = match cached {
                Cached::Prepared(index) => Some(Prepared {
                    index: index + 1,
                    source: self.sources[index].clone(),
                    cached: true,
                }),
                Cached::Unchanged => None,
            };
            self.record(
                &caller.symbol,
                &source.symbol,
                depth,
                source.live_count(),
                result.as_ref().map(|prepared| prepared.source.live_count()),
                "cache",
                None,
            );
            return Ok(result);
        }
        if self
            .active
            .iter()
            .any(|active| active.symbol == source.symbol)
        {
            self.record(
                &caller.symbol,
                &source.symbol,
                depth,
                source.live_count(),
                None,
                "cycle",
                Some(InlineSkip::Recursive),
            );
            return Ok(None);
        }
        self.active.push(Active {
            symbol: source.symbol.clone(),
            refinements: reapply::Refinements::new(key.facts.clone(), source.len()),
        });
        let result = self.prepare_new(caller, source, provider, costs, depth, &key);
        self.active.pop();
        result
    }

    fn prepare_new(
        &mut self,
        caller: &FunctionGraph,
        source: &FunctionGraph,
        provider: &(impl GraphProvider + ?Sized),
        costs: &mut Costs<'_>,
        depth: usize,
        key: &Key,
    ) -> Result<Option<Prepared>, InlineError> {
        let mut replacements = Vec::new();
        // Count the substitutions already selected below; diagnostics do not
        // add any fact propagation or a second optimization traversal.
        let mut fact_hits = self
            .dump
            .matches(&source.symbol)
            .then(|| vec![0; key.facts.len()]);
        self.stats.nodes_visited += source.live_count();
        for node in source.ids() {
            let Some((parameter, fields)) = parameter_field(source, node) else {
                continue;
            };
            let Some(ty) = source.rust_type(node) else {
                continue;
            };
            if let Some((index, fact)) = key.facts.iter().enumerate().find(|(_, fact)| {
                fact.parameter == parameter
                    && fact.fields == fields
                    && source.oracle.shape(ty) == &fact.shape
            }) {
                replacements.push((node, ty, fact.bits));
                if let Some(hits) = &mut fact_hits {
                    hits[index] += 1;
                }
            }
        }
        if replacements.is_empty() {
            self.cache.insert(key.clone(), Cached::Unchanged);
            return Ok(None);
        }
        if source.len() > self.remaining_nodes {
            self.limited();
            self.record(
                &caller.symbol,
                &source.symbol,
                depth,
                source.live_count(),
                None,
                "clone",
                Some(InlineSkip::Budget),
            );
            return Ok(None);
        }
        self.remaining_nodes -= source.len();
        self.nodes_cloned += source.len();
        self.stats.source_nodes_cloned += source.len();
        let mut candidate = source.clone();
        for (node, ty, bits) in replacements {
            let constant = candidate.add_constant(bits, ty);
            candidate.replace(node, constant);
        }
        candidate.resolve_edges();
        // Prune before a single child source is requested or copied. The local
        // cleanup retains NodeIds throughout all recursive preparation.
        if !self.clean(&mut candidate)? {
            return Ok(None);
        }
        let preparation = Preparation(provider);
        let _nested = super::driver::run(
            &mut candidate,
            &preparation as &dyn GraphProvider,
            InlineBudget {
                max_depth: MAX_DEPTH,
                // This is only a snapshot of the same account. The driver
                // rechecks its live remainder after recursive preparation and
                // reserves the complete transaction before every splice plan.
                max_nodes: self.remaining_nodes,
                max_calls: MAX_HELPERS,
                copy_anonymous_constants: true,
                ..InlineBudget::default()
            },
            self,
            costs,
            super::driver::Frame::Preparation { depth },
        )?;
        if !self.clean(&mut candidate)? {
            return Ok(None);
        }
        // Compaction retires preparation-queue NodeIds. Consumers see only the
        // final immutable source; diagnostics label its own local NodeIds, and
        // the outer plan rechecks every live reference.
        candidate.compact();
        crate::verify::verify_pipeline_step(&mut candidate).map_err(InlineError::Verification)?;
        if let Some(hits) = fact_hits {
            self.dump.emit(
                self.active
                    .first()
                    .map_or(&source.symbol, |root| &root.symbol),
                &caller.symbol,
                &candidate,
                depth,
                &key.facts,
                &hits,
            );
        }
        let index = self.sources.len();
        let candidate = Rc::new(candidate);
        self.record(
            &caller.symbol,
            &source.symbol,
            depth,
            source.live_count(),
            Some(candidate.live_count()),
            "prepared",
            None,
        );
        self.sources.push(candidate.clone());
        self.cache.insert(key.clone(), Cached::Prepared(index));
        Ok(Some(Prepared {
            index: index + 1,
            source: candidate,
            cached: false,
        }))
    }
}

fn collect(
    graph: &FunctionGraph,
    node: NodeId,
    parameter: u32,
    fields: &mut Vec<u32>,
    facts: &mut Vec<Fact>,
    fuel: &mut usize,
) {
    if *fuel == 0 || facts.len() == MAX_FACTS {
        return;
    }
    *fuel -= 1;
    let node = selected_value(graph, node, fuel);
    if !graph.control(node).is_none() {
        return;
    }
    if let Some(ty) = graph.rust_type(node) {
        let shape = graph.oracle.shape(ty);
        let bits = if graph.op(node) == Opcode::Const {
            Some(graph.constant_value(node))
        } else {
            crate::analysis::known_bits::of(graph, node)
                .and_then(|known| {
                    let (low, high) = known.unsigned_bounds();
                    (low == high).then_some(u128::from(low))
                })
                .or_else(|| facts::scalar_bits(graph, node, fuel))
        };
        if let Some(bits) = bits.filter(|_| {
            matches!(
                shape,
                TypeShape::Int { .. } | TypeShape::Bool | TypeShape::Char
            )
        }) {
            // A KnownBits fact holds whenever the original value is defined.
            // Splicing observes every original argument before entering the
            // refined body, including unselected aggregate fields.
            facts.push(Fact {
                parameter,
                fields: fields.clone(),
                shape: shape.clone(),
                bits,
            });
            return;
        }
    }
    if fields.len() < MAX_FIELD_DEPTH
        && matches!(graph.op(node), Opcode::Aggregate | Opcode::WidePtr)
    {
        for (field, &input) in graph.inputs(node).iter().enumerate() {
            if *fuel == 0 || facts.len() == MAX_FACTS {
                break;
            }
            fields.push(field as u32);
            collect(graph, input, parameter, fields, facts, fuel);
            fields.pop();
        }
    }
}

/// Read fields of explicit constructors without mutating the caller or
/// observing a different field. Splice forwarding can expose constructors
/// whose projections have not reached the caller's seam-cleanup pass yet.
fn selected_value(graph: &FunctionGraph, node: NodeId, fuel: &mut usize) -> NodeId {
    let node = graph.resolve(node);
    if *fuel == 0 || !graph.control(node).is_none() || graph.op(node) != Opcode::Field {
        return node;
    }
    *fuel -= 1;
    let source = selected_value(graph, graph.input(node, 0), fuel);
    if matches!(graph.op(source), Opcode::Aggregate | Opcode::WidePtr)
        && let Some(&input) = graph.inputs(source).get(graph.payload(node) as usize)
    {
        selected_value(graph, input, fuel)
    } else {
        node
    }
}

fn parameter_field(graph: &FunctionGraph, mut node: NodeId) -> Option<(u32, Vec<u32>)> {
    let mut fields = Vec::new();
    while graph.op(node) == Opcode::Field && fields.len() < MAX_FIELD_DEPTH {
        if !graph.control(node).is_none() {
            return None;
        }
        fields.push(graph.payload(node));
        node = graph.resolve(graph.input(node, 0));
    }
    if graph.op(node) != Opcode::Projection
        || graph.input(node, 0) != graph.start
        || graph.payload(node) < 2
    {
        return None;
    }
    fields.reverse();
    Some((graph.payload(node) - 2, fields))
}
