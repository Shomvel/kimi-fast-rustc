use crate::profile::json_string;
use kfrc_son::{FunctionGraph, InlineSkip, InlineStats, Opcode};
use std::collections::BTreeMap;

fn enabled() -> bool {
    std::env::var_os("KFRC_CGSCC_STATS").is_some()
        || std::env::var_os("KFRC_SPECIALIZATION_STATS").is_some()
        || std::env::var_os("KFRC_DUMP_GRAPH").is_some()
}

fn calls(graph: &FunctionGraph) -> usize {
    graph
        .ids()
        .filter(|&node| graph.op(node) == Opcode::Call)
        .count()
}

pub(super) struct Row {
    symbol: String,
    pub outcome: &'static str,
    pub source_nodes: usize,
    pub final_nodes: usize,
    pub source_calls: usize,
    pub final_calls: usize,
    pub candidate_nodes: usize,
    pub candidate_calls: usize,
    pub calls_inlined: usize,
    pub identity_calls_folded: usize,
    pub emission_inline_complete: bool,
    pub emission_cached: bool,
    pub emission_stored_nodes: usize,
    pub cached_emission_inline_complete: bool,
    pub emission_cache_budget: bool,
    skips: BTreeMap<&'static str, usize>,
}

impl Row {
    pub fn new(graph: &FunctionGraph) -> Self {
        let source_nodes = graph.live_count();
        let source_calls = calls(graph);
        Self {
            symbol: graph.symbol.clone(),
            outcome: "unchanged",
            source_nodes,
            final_nodes: source_nodes,
            source_calls,
            final_calls: source_calls,
            candidate_nodes: source_nodes,
            candidate_calls: source_calls,
            calls_inlined: 0,
            identity_calls_folded: 0,
            emission_inline_complete: false,
            emission_cached: false,
            emission_stored_nodes: 0,
            cached_emission_inline_complete: false,
            emission_cache_budget: false,
            skips: BTreeMap::new(),
        }
    }

    pub fn candidate(&mut self, graph: &FunctionGraph) {
        self.candidate_nodes = graph.live_count();
        self.candidate_calls = calls(graph);
    }

    pub fn record_skips(&mut self, stats: &InlineStats) {
        for (_, reason) in &stats.skipped {
            let name = match reason {
                InlineSkip::Missing => "missing",
                InlineSkip::Recursive => "recursive",
                InlineSkip::Budget => "budget",
                InlineSkip::Size => "size",
                InlineSkip::Cost => "cost",
                InlineSkip::UnknownCost => "unknown_cost",
                InlineSkip::Unsupported => "unsupported",
                InlineSkip::TypeConflict => "type_conflict",
                InlineSkip::AbiMismatch => "abi_mismatch",
                InlineSkip::DataConflict => "data_conflict",
                InlineSkip::PrivateReference => "private_reference",
            };
            *self.skips.entry(name).or_default() += 1;
        }
    }

    fn emit(&self) {
        if !enabled() {
            return;
        }
        let skips = self
            .skips
            .iter()
            .map(|(name, count)| format!("\"{name}\":{count}"))
            .collect::<Vec<_>>()
            .join(",");
        eprintln!(
            "kfrc-cgscc {{\"schema\":1,\"symbol\":{},\"outcome\":{},\"source_nodes\":{},\"final_nodes\":{},\"source_calls\":{},\"final_calls\":{},\"candidate_nodes\":{},\"candidate_calls\":{},\"calls_inlined\":{},\"identity_calls_folded\":{},\"emission_inline_complete\":{},\"emission_cached\":{},\"emission_stored_nodes\":{},\"cached_emission_inline_complete\":{},\"emission_cache_budget\":{},\"skips\":{{{skips}}}}}",
            json_string(&self.symbol),
            json_string(self.outcome),
            self.source_nodes,
            self.final_nodes,
            self.source_calls,
            self.final_calls,
            self.candidate_nodes,
            self.candidate_calls,
            self.calls_inlined,
            self.identity_calls_folded,
            self.emission_inline_complete,
            self.emission_cached,
            self.emission_stored_nodes,
            self.cached_emission_inline_complete,
            self.emission_cache_budget,
        );
    }
}

pub(super) struct Summary {
    pub source_functions: usize,
    pub reachable_functions: usize,
    pub components: usize,
    pub published: usize,
    pub unchanged: usize,
    pub recursive: usize,
    pub private_reference: usize,
    pub profitability_growth: usize,
    pub work_budget: usize,
    pub source_nodes: usize,
    pub final_nodes: usize,
    pub source_calls: usize,
    pub final_calls: usize,
    pub calls_inlined: usize,
    pub identity_calls_folded: usize,
    pub work_limit: usize,
    pub work_used: usize,
    pub emission_candidates: usize,
    pub emission_candidate_nodes: usize,
    pub emission_stored_nodes: usize,
    pub emission_cache_budget: usize,
}

impl Summary {
    pub fn new(
        source_functions: usize,
        reachable_functions: usize,
        components: usize,
        work_limit: usize,
    ) -> Self {
        Self {
            source_functions,
            reachable_functions,
            components,
            published: 0,
            unchanged: 0,
            recursive: 0,
            private_reference: 0,
            profitability_growth: 0,
            work_budget: 0,
            source_nodes: 0,
            final_nodes: 0,
            source_calls: 0,
            final_calls: 0,
            calls_inlined: 0,
            identity_calls_folded: 0,
            work_limit,
            work_used: 0,
            emission_candidates: 0,
            emission_candidate_nodes: 0,
            emission_stored_nodes: 0,
            emission_cache_budget: 0,
        }
    }

    pub fn record(&mut self, row: Row) {
        match row.outcome {
            "published" => self.published += 1,
            "recursive" => self.recursive += 1,
            "private_reference" => self.private_reference += 1,
            "profitability_growth" => self.profitability_growth += 1,
            "work_budget" => self.work_budget += 1,
            _ => self.unchanged += 1,
        }
        self.source_nodes += row.source_nodes;
        self.final_nodes += row.final_nodes;
        self.source_calls += row.source_calls;
        self.final_calls += row.final_calls;
        self.calls_inlined += row.calls_inlined;
        self.identity_calls_folded += row.identity_calls_folded;
        if row.emission_cached {
            self.emission_candidates += 1;
            self.emission_candidate_nodes += row.candidate_nodes;
            self.emission_stored_nodes += row.emission_stored_nodes;
        }
        self.emission_cache_budget += usize::from(row.emission_cache_budget);
        row.emit();
    }

    pub fn emit(&self) {
        if enabled() {
            eprintln!(
                "kfrc-cgscc-summary {{\"schema\":1,\"source_functions\":{},\"reachable_functions\":{},\"components\":{},\"published\":{},\"unchanged\":{},\"recursive\":{},\"private_reference\":{},\"profitability_growth\":{},\"work_budget\":{},\"source_nodes\":{},\"final_nodes\":{},\"source_calls\":{},\"final_calls\":{},\"calls_inlined\":{},\"identity_calls_folded\":{},\"work_limit\":{},\"work_used\":{},\"emission_candidates\":{},\"emission_candidate_nodes\":{},\"emission_stored_nodes\":{},\"emission_node_limit\":{},\"emission_cache_budget\":{}}}",
                self.source_functions,
                self.reachable_functions,
                self.components,
                self.published,
                self.unchanged,
                self.recursive,
                self.private_reference,
                self.profitability_growth,
                self.work_budget,
                self.source_nodes,
                self.final_nodes,
                self.source_calls,
                self.final_calls,
                self.calls_inlined,
                self.identity_calls_folded,
                self.work_limit,
                self.work_used,
                self.emission_candidates,
                self.emission_candidate_nodes,
                self.emission_stored_nodes,
                0,
                self.emission_cache_budget,
            );
        }
    }
}
