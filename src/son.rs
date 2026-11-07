use std::error::Error;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeId(pub u32);

impl NodeId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "n{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputChannel {
    Control,
    Data,
    Memory,
}

/// `Neg` wraps on overflow; `Not` is bitwise inversion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnaryOp {
    Neg,
    Not,
}

/// Arithmetic wraps, except `Div` and `Rem`, which trap on zero divisors and
/// `i64::MIN / -1`. Shifts mask the count to six bits; `Shr` is arithmetic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    And,
    Or,
    Xor,
    Shl,
    Shr,
}

/// Signed comparisons produce an `i64` zero or one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeKind {
    Start,
    Const(i64),
    Parameter { index: usize },
    Unary(UnaryOp),
    Binary(BinaryOp),
    Compare(CompareOp),
    Call(usize),
    If,
    Projection(u8),
    Region,
    Jump,
    Phi,
    Return,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Node {
    pub kind: NodeKind,
    pub control: Vec<NodeId>,
    pub data: Vec<NodeId>,
    /// Reserved for memory-state tokens. Optimizations preserve these edges.
    pub memory: Vec<NodeId>,
}

impl Node {
    pub fn new(kind: NodeKind) -> Self {
        Self {
            kind,
            control: Vec::new(),
            data: Vec::new(),
            memory: Vec::new(),
        }
    }

    pub fn with_inputs(kind: NodeKind, control: Vec<NodeId>, data: Vec<NodeId>) -> Self {
        Self {
            kind,
            control,
            data,
            memory: Vec::new(),
        }
    }

    pub fn with_memory_inputs(
        kind: NodeKind,
        control: Vec<NodeId>,
        data: Vec<NodeId>,
        memory: Vec<NodeId>,
    ) -> Self {
        Self {
            kind,
            control,
            data,
            memory,
        }
    }

    pub fn inputs(&self, channel: InputChannel) -> &[NodeId] {
        match channel {
            InputChannel::Control => &self.control,
            InputChannel::Data => &self.data,
            InputChannel::Memory => &self.memory,
        }
    }

    pub fn inputs_mut(&mut self, channel: InputChannel) -> &mut Vec<NodeId> {
        match channel {
            InputChannel::Control => &mut self.control,
            InputChannel::Data => &mut self.data,
            InputChannel::Memory => &mut self.memory,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidationError {
    MissingInput {
        node: NodeId,
        channel: InputChannel,
        index: usize,
        input: NodeId,
    },
    WrongArity {
        node: NodeId,
        kind: NodeKind,
        channel: InputChannel,
        expected: &'static str,
        actual: usize,
    },
    PhiInputCount {
        node: NodeId,
        controls: usize,
        values: usize,
    },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingInput {
                node,
                channel,
                index,
                input,
            } => write!(
                f,
                "{node} has missing {channel:?} input {input} at position {index}"
            ),
            Self::WrongArity {
                node,
                kind,
                channel,
                expected,
                actual,
            } => write!(
                f,
                "{node} ({kind:?}) expects {expected} {channel:?} inputs, found {actual}"
            ),
            Self::PhiInputCount {
                node,
                controls,
                values,
            } => write!(
                f,
                "{node} has {controls} control inputs but {values} data inputs"
            ),
        }
    }
}

impl Error for ValidationError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FunctionMetadata {
    pub name: String,
    pub start: NodeId,
    pub return_node: NodeId,
    pub parameter_count: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Graph {
    nodes: Vec<Option<Node>>,
    live_nodes: usize,
    pub functions: Vec<FunctionMetadata>,
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.live_nodes
    }

    pub fn is_empty(&self) -> bool {
        self.live_nodes == 0
    }

    pub fn add_node(&mut self, kind: NodeKind, control: Vec<NodeId>, data: Vec<NodeId>) -> NodeId {
        self.add_node_with_memory(kind, control, data, Vec::new())
    }

    pub fn add_node_with_memory(
        &mut self,
        kind: NodeKind,
        control: Vec<NodeId>,
        data: Vec<NodeId>,
        memory: Vec<NodeId>,
    ) -> NodeId {
        self.push(Node::with_memory_inputs(kind, control, data, memory))
    }

    pub fn push(&mut self, node: Node) -> NodeId {
        let index = u32::try_from(self.nodes.len()).expect("sea-of-nodes graph exceeds u32 IDs");
        self.nodes.push(Some(node));
        self.live_nodes += 1;
        NodeId(index)
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.index())?.as_ref()
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id.index())?.as_mut()
    }

    pub fn remove(&mut self, id: NodeId) -> Option<Node> {
        let removed = self.nodes.get_mut(id.index())?.take();
        if removed.is_some() {
            self.live_nodes -= 1;
        }
        removed
    }

    pub fn ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| node.as_ref().map(|_| NodeId(index as u32)))
    }

    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Node)> + '_ {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| node.as_ref().map(|node| (NodeId(index as u32), node)))
    }

    pub fn function_graph(&self, function: &FunctionMetadata) -> Graph {
        let mut graph = self.clone();
        let outside = graph
            .ids()
            .filter(|id| id.0 < function.start.0 || id.0 > function.return_node.0)
            .collect::<Vec<_>>();
        for id in outside {
            graph.remove(id);
        }
        graph
    }

    pub fn start(&mut self) -> NodeId {
        self.add_node(NodeKind::Start, Vec::new(), Vec::new())
    }

    pub fn constant(&mut self, value: i64) -> NodeId {
        self.add_node(NodeKind::Const(value), Vec::new(), Vec::new())
    }

    pub fn parameter(&mut self, index: usize) -> NodeId {
        self.add_node(NodeKind::Parameter { index }, Vec::new(), Vec::new())
    }

    pub fn unary(&mut self, op: UnaryOp, value: NodeId) -> NodeId {
        self.add_node(NodeKind::Unary(op), Vec::new(), vec![value])
    }

    pub fn binary(&mut self, op: BinaryOp, left: NodeId, right: NodeId) -> NodeId {
        self.add_node(NodeKind::Binary(op), Vec::new(), vec![left, right])
    }

    pub fn compare(&mut self, op: CompareOp, left: NodeId, right: NodeId) -> NodeId {
        self.add_node(NodeKind::Compare(op), Vec::new(), vec![left, right])
    }

    pub fn if_(&mut self, control: NodeId, condition: NodeId) -> NodeId {
        self.add_node(NodeKind::If, vec![control], vec![condition])
    }

    pub fn projection(&mut self, control: NodeId, index: u8) -> NodeId {
        self.add_node(NodeKind::Projection(index), vec![control], Vec::new())
    }

    pub fn region(&mut self, control: Vec<NodeId>) -> NodeId {
        self.add_node(NodeKind::Region, control, Vec::new())
    }

    pub fn jump(&mut self, control: NodeId) -> NodeId {
        self.add_node(NodeKind::Jump, vec![control], Vec::new())
    }

    pub fn phi(&mut self, control: Vec<NodeId>, data: Vec<NodeId>) -> NodeId {
        self.add_node(NodeKind::Phi, control, data)
    }

    pub fn return_(&mut self, control: NodeId, value: Option<NodeId>) -> NodeId {
        self.add_node(NodeKind::Return, vec![control], value.into_iter().collect())
    }

    pub fn replace_uses(&mut self, old: NodeId, new: NodeId) -> usize {
        if old == new {
            return 0;
        }

        let mut replaced = 0;
        for node in self.nodes.iter_mut().flatten() {
            for channel in [
                InputChannel::Control,
                InputChannel::Data,
                InputChannel::Memory,
            ] {
                for input in node.inputs_mut(channel) {
                    if *input == old {
                        *input = new;
                        replaced += 1;
                    }
                }
            }
        }
        replaced
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        for (id, node) in self.iter() {
            for channel in [
                InputChannel::Control,
                InputChannel::Data,
                InputChannel::Memory,
            ] {
                for (index, &input) in node.inputs(channel).iter().enumerate() {
                    if self.node(input).is_none() {
                        return Err(ValidationError::MissingInput {
                            node: id,
                            channel,
                            index,
                            input,
                        });
                    }
                }
            }

            match node.kind {
                NodeKind::Start | NodeKind::Const(_) | NodeKind::Parameter { .. } => {
                    expect_arity(id, node.kind, InputChannel::Control, 0, node.control.len())?;
                    expect_arity(id, node.kind, InputChannel::Data, 0, node.data.len())?;
                }
                NodeKind::Unary(_) => {
                    expect_arity(id, node.kind, InputChannel::Control, 0, node.control.len())?;
                    expect_arity(id, node.kind, InputChannel::Data, 1, node.data.len())?;
                }
                NodeKind::Binary(_) | NodeKind::Compare(_) => {
                    expect_arity(id, node.kind, InputChannel::Control, 0, node.control.len())?;
                    expect_arity(id, node.kind, InputChannel::Data, 2, node.data.len())?;
                }
                NodeKind::Call(_) => {
                    expect_arity(id, node.kind, InputChannel::Control, 1, node.control.len())?;
                }
                NodeKind::If => {
                    expect_arity(id, node.kind, InputChannel::Control, 1, node.control.len())?;
                    expect_arity(id, node.kind, InputChannel::Data, 1, node.data.len())?;
                }
                NodeKind::Projection(_) | NodeKind::Jump => {
                    expect_arity(id, node.kind, InputChannel::Control, 1, node.control.len())?;
                    expect_arity(id, node.kind, InputChannel::Data, 0, node.data.len())?;
                }
                NodeKind::Region => {
                    if node.control.is_empty() {
                        return Err(ValidationError::WrongArity {
                            node: id,
                            kind: node.kind,
                            channel: InputChannel::Control,
                            expected: "at least 1",
                            actual: 0,
                        });
                    }
                    expect_arity(id, node.kind, InputChannel::Data, 0, node.data.len())?;
                }
                NodeKind::Phi => {
                    if node.control.is_empty() {
                        return Err(ValidationError::WrongArity {
                            node: id,
                            kind: node.kind,
                            channel: InputChannel::Control,
                            expected: "at least 1",
                            actual: 0,
                        });
                    }
                    if node.control.len() != node.data.len() {
                        return Err(ValidationError::PhiInputCount {
                            node: id,
                            controls: node.control.len(),
                            values: node.data.len(),
                        });
                    }
                }
                NodeKind::Return => {
                    expect_arity(id, node.kind, InputChannel::Control, 1, node.control.len())?;
                    if node.data.len() > 1 {
                        return Err(ValidationError::WrongArity {
                            node: id,
                            kind: node.kind,
                            channel: InputChannel::Data,
                            expected: "0 or 1",
                            actual: node.data.len(),
                        });
                    }
                }
            }
        }
        Ok(())
    }
}

fn expect_arity(
    node: NodeId,
    kind: NodeKind,
    channel: InputChannel,
    expected: usize,
    actual: usize,
) -> Result<(), ValidationError> {
    if expected == actual {
        Ok(())
    } else {
        Err(ValidationError::WrongArity {
            node,
            kind,
            channel,
            expected: match expected {
                0 => "0",
                1 => "1",
                2 => "2",
                _ => unreachable!(),
            },
            actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_and_validates_a_function_graph() {
        let mut graph = Graph::new();
        let start = graph.start();
        let left = graph.parameter(0);
        let right = graph.parameter(1);
        let sum = graph.binary(BinaryOp::Add, left, right);
        let zero = graph.constant(0);
        let positive = graph.compare(CompareOp::Gt, sum, zero);
        let phi = graph.phi(vec![start], vec![positive]);
        let result = graph.return_(start, Some(phi));

        graph.validate().unwrap();
        assert_eq!(graph.len(), 8);
        assert_eq!(graph.node(result).unwrap().control, vec![start]);
        assert_eq!(graph.node(sum).unwrap().data, vec![left, right]);
        assert_eq!(
            graph.node(left).unwrap().kind,
            NodeKind::Parameter { index: 0 }
        );
    }

    #[test]
    fn validation_reports_missing_inputs_and_bad_arities() {
        let mut graph = Graph::new();
        let missing = NodeId(99);
        let add = graph.binary(BinaryOp::Add, missing, missing);
        assert_eq!(
            graph.validate(),
            Err(ValidationError::MissingInput {
                node: add,
                channel: InputChannel::Data,
                index: 0,
                input: missing,
            })
        );

        let mut graph = Graph::new();
        let start = graph.start();
        let value = graph.constant(1);
        let phi = graph.phi(vec![start, start], vec![value]);
        assert_eq!(
            graph.validate(),
            Err(ValidationError::PhiInputCount {
                node: phi,
                controls: 2,
                values: 1,
            })
        );

        graph.remove(phi);
        let bad_return = graph.add_node(NodeKind::Return, Vec::new(), Vec::new());
        assert!(matches!(
            graph.validate(),
            Err(ValidationError::WrongArity { node, .. }) if node == bad_return
        ));
    }

    #[test]
    fn replaces_control_data_and_memory_uses() {
        let mut graph = Graph::new();
        let old_start = graph.start();
        let new_start = graph.start();
        let old_value = graph.constant(1);
        let new_value = graph.constant(2);
        let result = graph.add_node_with_memory(
            NodeKind::Return,
            vec![old_start],
            vec![old_value],
            vec![old_value],
        );

        assert_eq!(graph.replace_uses(old_value, new_value), 2);
        assert_eq!(graph.replace_uses(old_start, new_start), 1);
        assert_eq!(graph.replace_uses(old_value, old_value), 0);
        let node = graph.node(result).unwrap();
        assert_eq!(node.control, vec![new_start]);
        assert_eq!(node.data, vec![new_value]);
        assert_eq!(node.memory, vec![new_value]);
        graph.validate().unwrap();
    }

    #[test]
    fn removal_leaves_other_ids_stable() {
        let mut graph = Graph::new();
        let first = graph.constant(1);
        let second = graph.constant(2);

        assert_eq!(graph.remove(first).unwrap().kind, NodeKind::Const(1));
        assert_eq!(graph.remove(first), None);
        assert_eq!(graph.node(first), None);
        assert_eq!(graph.node(second).unwrap().kind, NodeKind::Const(2));
        assert_eq!(graph.ids().collect::<Vec<_>>(), vec![second]);
        assert_eq!(graph.len(), 1);
    }
}
