use super::{UnrollStats, plan::Plan};
use crate::transforms::utils::cfg::{CfgEditError, CfgEditor};
use crate::{FunctionGraph, NodeId};
use kfrc_body::fx_hash::HashMap;

impl Plan {
    fn mapped(
        &self,
        graph: &FunctionGraph,
        mut source: NodeId,
        mapping: &HashMap<NodeId, NodeId>,
    ) -> NodeId {
        loop {
            source = graph.resolve(source);
            if let Some(&replacement) = mapping.get(&source) {
                return replacement;
            }
            if let Some(&alias) = self.aliases.get(&source) {
                source = alias;
            } else {
                return source;
            }
        }
    }

    pub fn apply(
        self,
        graph: &mut FunctionGraph,
        stats: &mut UnrollStats,
    ) -> Result<(), CfgEditError> {
        let before = graph.len();
        let iterations = self.induction.values.len() - 1;
        let initial_iv = self
            .phis
            .iter()
            .find(|phi| phi.node == self.induction.phi)
            .unwrap()
            .entry;
        let mut induction = vec![graph.resolve(initial_iv)];
        induction.extend(
            self.induction.values[1..]
                .iter()
                .map(|&value| graph.add_constant(u128::from(value), self.induction.ty)),
        );
        let mut state: Vec<_> = self
            .phis
            .iter()
            .map(|phi| graph.resolve(phi.entry))
            .collect();
        let mut control = self.header;
        for iteration in 0..iterations {
            let mut mapping: HashMap<_, _> = self
                .phis
                .iter()
                .zip(&state)
                .map(|(phi, &value)| (phi.node, value))
                .collect();
            mapping.insert(self.header, control);
            mapping.insert(self.induction.phi, induction[iteration]);
            // Allocate every result before wiring operands; value definitions
            // need not be stored before their users in the source graph.
            for &source in &self.copies {
                let copy = graph.add_node(
                    graph.op(source),
                    NodeId::NONE,
                    &[],
                    graph.payload(source),
                    graph.value_type(source).clone(),
                );
                mapping.insert(source, copy);
            }
            for &source in &self.copies {
                let copy = mapping[&source];
                let control = self.mapped(graph, graph.control(source), &mapping);
                let inputs: Vec<_> = graph
                    .inputs(source)
                    .iter()
                    .map(|&input| self.mapped(graph, input, &mapping))
                    .collect();
                graph.set_control(copy, control);
                graph.set_inputs(copy, &inputs);
            }
            // Compute the whole next-state vector against the same previous
            // vector. This includes effect and mutually dependent value Phis.
            state = self
                .phis
                .iter()
                .map(|phi| {
                    if phi.node == self.induction.phi {
                        induction[iteration + 1]
                    } else {
                        self.mapped(graph, phi.backedge, &mapping)
                    }
                })
                .collect();
            control = self.mapped(graph, self.latch, &mapping);
        }
        for (phi, value) in self.phis.iter().zip(state) {
            graph.replace(phi.node, value);
        }
        graph.replace(self.exit, control);
        graph.remove_node(self.branch);
        CfgEditor::new(graph)
            .retain_predecessors(self.header, &[self.entry_slot == 0, self.entry_slot == 1])?;
        debug_assert_eq!(graph.len() - before, self.cost());
        stats.loops_unrolled += 1;
        stats.iterations_cloned += iterations;
        stats.nodes_cloned += graph.len() - before;
        stats
            .dirty_nodes
            .extend((before..graph.len()).map(|i| NodeId(i as u32)));
        stats.dirty_nodes.extend(self.consumers);
        stats.dirty_nodes.push(self.header);
        stats.exposed_slots.extend(self.slots);
        Ok(())
    }
}
