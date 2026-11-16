use super::*;

pub(super) struct Values<'a> {
    pub graph: &'a mut FunctionGraph,
    cfg: &'a ControlFlow,
    slot: &'a Slot,
    lane: usize,
    ty: TyId,
    undefined: NodeId,
    values: HashMap<NodeId, NodeId>,
    pub phis: Vec<NodeId>,
}

enum Work {
    Read(NodeId),
    Forward(NodeId, NodeId),
    Phi(NodeId, NodeId),
}

impl<'a> Values<'a> {
    pub fn new(
        graph: &'a mut FunctionGraph,
        cfg: &'a ControlFlow,
        slot: &'a Slot,
        lane: usize,
        ty: TyId,
    ) -> Self {
        let undefined =
            graph.add_node(Opcode::Undefined, NodeId::NONE, &[], 0, ValueType::Rust(ty));
        Self {
            graph,
            cfg,
            slot,
            lane,
            ty,
            undefined,
            values: HashMap::default(),
            phis: Vec::new(),
        }
    }

    /// A Region placeholder is installed before traversing its incoming
    /// edges, so loop backedges terminate without a recursive Rust stack.
    pub fn read(&mut self, node: NodeId) -> NodeId {
        let mut pending = vec![Work::Read(node)];
        while let Some(work) = pending.pop() {
            match work {
                Work::Read(node) => {
                    if self.values.contains_key(&node) {
                        continue;
                    }
                    if node == self.graph.start {
                        self.values.insert(node, self.undefined);
                    } else if self.graph.op(node) == Opcode::Region {
                        let incoming = self.graph.inputs(node).to_vec();
                        let phi = self.graph.add_growable_node(
                            Opcode::Phi,
                            NodeId::NONE,
                            incoming.len() + 1,
                            0,
                            ValueType::Rust(self.ty),
                        );
                        self.graph.set_input(phi, 0, node);
                        self.phis.push(phi);
                        self.values.insert(node, phi);
                        pending.push(Work::Phi(node, phi));
                        pending.extend(incoming.into_iter().map(Work::Read));
                    } else if let Some(lane) = self.slot.accesses.get(&node)
                        && (lane.is_none()
                            || *lane == Some(self.lane) && self.graph.op(node) == Opcode::Store)
                    {
                        let value = if lane.is_none() {
                            self.undefined
                        } else {
                            self.graph.input(node, 2)
                        };
                        self.values.insert(node, value);
                    } else {
                        let index = self.cfg.position[&node];
                        let before = self.cfg.order[self.cfg.predecessors[index][0]];
                        pending.push(Work::Forward(node, before));
                        pending.push(Work::Read(before));
                    }
                }
                Work::Forward(node, before) => {
                    self.values.insert(node, self.values[&before]);
                }
                Work::Phi(region, phi) => {
                    let incoming = self.graph.inputs(region).to_vec();
                    for (index, before) in incoming.into_iter().enumerate() {
                        self.graph.set_input(phi, index + 1, self.values[&before]);
                    }
                    // Ordinary scalar cleanup handles equal/multi-input Phis.
                    if self.graph.inputs(phi).len() == 2 {
                        let value = self.graph.resolve(self.graph.input(phi, 1));
                        if value != phi {
                            self.graph.replace(phi, value);
                        }
                    }
                }
            }
        }
        self.values[&node]
    }
}
