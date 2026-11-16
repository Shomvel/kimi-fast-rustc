//! Bounded profitability hints over immutable S graphs, never legality facts.
//! Preparation stays context independent. Emission inherits hot callers only
//! through classified non-cold paths, including calls from prepared sources.
use crate::analysis::cfg::ControlFlow;
use crate::analysis::cost::GraphCost;
use crate::{FunctionGraph, InlineCostPolicy, InlineTemperature, NodeId, Opcode};
use kfrc_body::fx_hash::HashMap;

const WORK_LIMIT: usize = 100_000;

pub(super) struct Pricing {
    enabled: bool,
    remaining: usize,
    pub work: usize,
    pub bailouts: usize,
    hot_remaining: u64,
    sites: HashMap<NodeId, InlineTemperature>,
    sources: HashMap<(String, usize), Option<HashMap<NodeId, InlineTemperature>>>,
}

impl Pricing {
    pub fn new(policy: InlineCostPolicy, preparation: bool, original: &FunctionGraph) -> Self {
        let mut pricing = Self {
            enabled: !preparation && matches!(policy, InlineCostPolicy::CallSiteResidual { .. }),
            remaining: WORK_LIMIT,
            work: 0,
            bailouts: 0,
            hot_remaining: match policy {
                InlineCostPolicy::CallSiteResidual {
                    total_hot_growth, ..
                } => total_hot_growth,
                _ => 0,
            },
            sites: HashMap::default(),
            sources: HashMap::default(),
        };
        if pricing.enabled && original.ids().any(|node| original.op(node) == Opcode::Call) {
            pricing.sites = pricing.classify(original).unwrap_or_default();
        }
        pricing
    }

    pub fn temperature(&self, call: NodeId) -> InlineTemperature {
        self.sites
            .get(&call)
            .copied()
            .unwrap_or(InlineTemperature::Unknown)
    }

    /// Normalize the configured policy before all shared inliner decisions.
    /// Disabled preparation has Unknown sites and therefore the base limit.
    pub fn policy(&self, policy: InlineCostPolicy, call: NodeId) -> InlineCostPolicy {
        match policy {
            InlineCostPolicy::CallSiteResidual {
                max_growth,
                hot_growth,
                ..
            } => InlineCostPolicy::Residual {
                max_growth: if self.temperature(call) == InlineTemperature::Hot {
                    hot_growth.min(max_growth.saturating_add(self.hot_remaining))
                } else {
                    max_growth
                },
            },
            policy => policy,
        }
    }

    pub fn hot_growth_remaining(&self) -> Option<u64> {
        self.enabled.then_some(self.hot_remaining)
    }

    /// The caller commits this charge only after a successful splice. Sole
    /// references and strong inline intent use their existing exceptions and
    /// never call this method. The ordinary threshold is not charged again.
    pub fn charge(
        &mut self,
        policy: InlineCostPolicy,
        call: NodeId,
        estimate: Option<GraphCost>,
        call_units: Option<u64>,
    ) -> u64 {
        let InlineCostPolicy::CallSiteResidual { max_growth, .. } = policy else {
            return 0;
        };
        if !self.enabled || self.temperature(call) != InlineTemperature::Hot {
            return 0;
        }
        let charge = estimate
            .and_then(|cost| cost.code_size.total())
            .zip(call_units)
            .map_or(0, |(source, call)| {
                source.saturating_sub(call).saturating_sub(max_growth)
            });
        debug_assert!(charge <= self.hot_remaining);
        self.hot_remaining = self.hot_remaining.saturating_sub(charge);
        charge
    }

    pub fn inherit(
        &mut self,
        parent: InlineTemperature,
        key: &(String, usize),
        source: &FunctionGraph,
        nested: &[(NodeId, NodeId)],
    ) {
        if !self.enabled || nested.is_empty() {
            return;
        }
        if parent != InlineTemperature::Cold && !self.sources.contains_key(key) {
            let sites = self.classify(source);
            self.sources.insert(key.clone(), sites);
        }
        let source_sites = self.sources.get(key).and_then(Option::as_ref);
        for &(original, copied) in nested {
            let local = source_sites
                .and_then(|sites| sites.get(&original))
                .copied()
                .unwrap_or(InlineTemperature::Unknown);
            let inherited = match (parent, local) {
                (InlineTemperature::Cold, _) | (_, InlineTemperature::Cold) => {
                    InlineTemperature::Cold
                }
                // Unknown source paths never inherit a hot threshold.
                (_, InlineTemperature::Unknown) => InlineTemperature::Unknown,
                (InlineTemperature::Hot, _) | (_, InlineTemperature::Hot) => InlineTemperature::Hot,
                _ => InlineTemperature::Ordinary,
            };
            self.sites.insert(copied, inherited);
        }
    }

    fn classify(&mut self, graph: &FunctionGraph) -> Option<HashMap<NodeId, InlineTemperature>> {
        // Reserve a size-based work charge before building a CFG or dense
        // vectors. len() includes dead slots; inputs bound the CFG edges.
        // The four-unit weight is a policy account, not an instruction count.
        let available = self.remaining / 4;
        let mut units = graph.len();
        if units <= available {
            for node in graph.ids() {
                units = units.saturating_add(graph.inputs(node).len());
                if units > available {
                    break;
                }
            }
        }
        if units > available {
            self.work += self.remaining;
            self.remaining = 0;
            self.bailouts += 1;
            return None;
        }
        units *= 4;
        self.remaining -= units;
        self.work += units;
        let cfg = ControlFlow::new(graph);
        let count = cfg.order.len();
        let mut cold = vec![false; count];
        let mut outgoing: Vec<_> = cfg
            .order
            .iter()
            .map(|&node| cfg.successors(node).len())
            .collect();
        let mut pending = Vec::new();
        for (index, &node) in cfg.order.iter().enumerate() {
            if graph.op(node) == Opcode::Unreachable {
                cold[index] = true;
                pending.push(index);
            }
        }
        while let Some(index) = pending.pop() {
            for &predecessor in &cfg.predecessors[index] {
                outgoing[predecessor] -= 1;
                if outgoing[predecessor] == 0 && !cold[predecessor] {
                    cold[predecessor] = true;
                    pending.push(predecessor);
                }
            }
        }
        // ControlFlow::order already is reverse postorder of the forward
        // graph. The reverse walk in that order is Kosaraju's second pass.
        // Iterative traversal also covers irreducible cycles without recursion.
        let mut seen = vec![false; count];
        let mut hot = vec![false; count];
        for root in 0..count {
            if seen[root] {
                continue;
            }
            seen[root] = true;
            pending.push(root);
            let mut members = Vec::new();
            while let Some(index) = pending.pop() {
                members.push(index);
                for &predecessor in &cfg.predecessors[index] {
                    if !seen[predecessor] {
                        seen[predecessor] = true;
                        pending.push(predecessor);
                    }
                }
            }
            if members.len() > 1 || cfg.successors(cfg.order[root]).contains(&cfg.order[root]) {
                for index in members {
                    hot[index] = true;
                }
            }
        }
        Some(
            graph
                .ids()
                .filter(|&node| graph.op(node) == Opcode::Call)
                .map(|node| {
                    let temperature = match cfg.position.get(&node).copied() {
                        None => InlineTemperature::Cold,
                        Some(index) if cold[index] => InlineTemperature::Cold,
                        Some(index) if hot[index] => InlineTemperature::Hot,
                        _ => InlineTemperature::Ordinary,
                    };
                    (node, temperature)
                })
                .collect(),
        )
    }
}
