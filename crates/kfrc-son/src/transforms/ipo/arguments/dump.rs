//! Explicit, bounded diagnostics for final scratch candidates. Every line has
//! its own prefix so emitted-graph parsers cannot mistake this for an S graph.
use super::Fact;
use crate::FunctionGraph;
use std::fmt::Write;

const MAX_GRAPH_NODES: usize = 1024;
const TOTAL_NODES: usize = 8192;
const TOTAL_BYTES: usize = 2 * 1024 * 1024;
const PREFIX: &str = "prep-graph|";

pub(super) struct Dump {
    filter: Option<String>,
    remaining_nodes: usize,
    remaining_bytes: usize,
}

impl Dump {
    pub(super) fn new() -> Self {
        Self {
            // Read once per Arguments session. Empty/non-Unicode filters keep
            // diagnostics disabled; no symbol has special meaning here.
            filter: std::env::var("KFRC_DUMP_INLINE_PREPARATION")
                .ok()
                .filter(|filter| !filter.is_empty()),
            remaining_nodes: TOTAL_NODES,
            remaining_bytes: TOTAL_BYTES,
        }
    }

    pub(super) fn matches(&self, symbol: &str) -> bool {
        self.remaining_nodes != 0
            && self.remaining_bytes != 0
            && self
                .filter
                .as_ref()
                .is_some_and(|filter| symbol.contains(filter))
    }

    pub(super) fn emit(
        &mut self,
        root: &str,
        owner: &str,
        graph: &FunctionGraph,
        depth: usize,
        facts: &[Fact],
        hits: &[usize],
    ) {
        if let Some(output) = self.render(root, owner, graph, depth, facts, hits) {
            eprint!("{output}");
        }
    }

    fn render(
        &mut self,
        root: &str,
        owner: &str,
        graph: &FunctionGraph,
        depth: usize,
        facts: &[Fact],
        hits: &[usize],
    ) -> Option<String> {
        let nodes = graph.len();
        if !self.matches(&graph.symbol) || nodes > MAX_GRAPH_NODES || nodes > self.remaining_nodes {
            return None;
        }
        // Rendering work is never refunded, including an over-byte-budget
        // result. These accounts grant no optimization or clone fuel.
        self.remaining_nodes -= nodes;
        let mut output = String::new();
        let _ = writeln!(
            output,
            "{PREFIX}begin root={root:?} owner={owner:?} symbol={:?} depth={depth} nodes={nodes} facts={} hits={} node_ids=scratch-local hit_scope=source-replacements-before-cleanup",
            graph.symbol,
            facts.len(),
            hits.iter().sum::<usize>(),
        );
        for (fact, hits) in facts.iter().zip(hits) {
            let _ = writeln!(
                output,
                "{PREFIX}fact parameter={} fields={:?} shape={:?} bits={} hits={hits}",
                fact.parameter, fact.fields, fact.shape, fact.bits,
            );
        }
        for line in graph.dump().lines() {
            let _ = writeln!(output, "{PREFIX}{line}");
        }
        let _ = writeln!(
            output,
            "{PREFIX}end root={root:?} owner={owner:?} symbol={:?} depth={depth} node_ids=scratch-local",
            graph.symbol,
        );
        if output.len() > self.remaining_bytes {
            self.remaining_bytes = 0;
            return None;
        }
        self.remaining_bytes -= output.len();
        Some(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeId, Opcode, ValueType};
    use kfrc_body::{FunctionAbi, PassMode, RecordedTypes, TypeShape};

    fn graph() -> FunctionGraph {
        let mut graph = FunctionGraph::new(
            "candidate\nname",
            FunctionAbi {
                args: Vec::new(),
                ret: PassMode::Ignore,
                c_variadic: false,
            },
            RecordedTypes::new(),
        );
        graph.start = graph.add_node(Opcode::Start, NodeId::NONE, &[], 0, ValueType::Control);
        graph
    }

    #[test]
    fn every_line_marks_scratch_scope_and_records_actual_fact_hits() {
        let graph = graph();
        let mut dump = Dump {
            filter: Some("candidate".into()),
            remaining_nodes: 10,
            remaining_bytes: 8192,
        };
        let facts = [Fact {
            parameter: 2,
            fields: vec![1, 0],
            shape: TypeShape::Bool,
            bits: 1,
        }];
        let output = dump
            .render("root", "owner", &graph, 3, &facts, &[2])
            .unwrap();
        assert!(output.lines().all(|line| line.starts_with(PREFIX)));
        assert!(output.contains("parameter=2 fields=[1, 0] shape=Bool bits=1 hits=2"));
        assert_eq!(output.matches("node_ids=scratch-local").count(), 2);
        assert_eq!(dump.remaining_nodes, 10 - graph.len());
        assert_eq!(dump.remaining_bytes, 8192 - output.len());
    }

    #[test]
    fn filtering_and_output_limits_do_not_refund_rendering_work() {
        let graph = graph();
        let mut dump = Dump {
            filter: None,
            remaining_nodes: 10,
            remaining_bytes: 8192,
        };
        assert!(dump.render("root", "owner", &graph, 0, &[], &[]).is_none());
        assert_eq!(dump.remaining_nodes, 10);
        dump.filter = Some("candidate".into());
        dump.remaining_nodes = 0;
        assert!(dump.render("root", "owner", &graph, 0, &[], &[]).is_none());
        dump.remaining_nodes = 10;
        dump.remaining_bytes = 1;
        assert!(dump.render("root", "owner", &graph, 0, &[], &[]).is_none());
        assert_eq!(dump.remaining_nodes, 10 - graph.len());
        assert_eq!(dump.remaining_bytes, 0);
        assert!(!dump.matches(&graph.symbol));
    }
}
