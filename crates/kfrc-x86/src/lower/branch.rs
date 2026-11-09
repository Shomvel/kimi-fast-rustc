//! Cover a Boolean comparison used only by its block's terminating branch.
//! The comparison is emitted at the terminator, after all operand definitions.
//! No load, call, observation, or shared Boolean value is covered or recomputed.
//!
//! A defined `bool` is exactly 0 or 1, so a chain of Boolean negations ending
//! at any other Boolean value branches on that value against zero with the
//! condition inverted once per negation.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct DirectBranch {
    pub lhs: NodeId,
    /// `None` compares `lhs` against zero.
    pub rhs: Option<NodeId>,
    pub condition: Condition,
}

pub(super) struct Branches {
    direct: HashMap<NodeId, DirectBranch>,
    covered: HashSet<NodeId>,
}

impl Branches {
    pub(super) fn new(graph: &FunctionGraph, schedule: &Schedule) -> Self {
        let mut uses = vec![0usize; graph.len()];
        for node in graph.ids() {
            for &input in graph.inputs(node) {
                uses[graph.resolve(input).index()] += 1;
            }
            let control = graph.control(node);
            if !control.is_none() {
                uses[graph.resolve(control).index()] += 1;
            }
        }
        let mut placement = vec![None; graph.len()];
        for (block, nodes) in schedule.placed.iter().enumerate() {
            for &node in nodes {
                placement[node.index()] = Some(block);
            }
        }
        let mut result = Self {
            direct: HashMap::default(),
            covered: HashSet::default(),
        };
        for (block, info) in schedule.blocks.iter().enumerate() {
            let Some(&branch) = info.chain.last() else {
                continue;
            };
            if graph.op(branch) != Opcode::If {
                continue;
            }
            let mut node = graph.resolve(graph.input(branch, 0));
            let mut inverted = false;
            let mut covered = Vec::new();
            loop {
                let boolean = graph
                    .rust_type(node)
                    .is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Bool));
                let coverable = boolean
                    && uses[node.index()] == 1
                    && placement[node.index()] == Some(block)
                    && graph.control(node).is_none();
                if coverable && graph.op(node) == Opcode::BitNot {
                    covered.push(node);
                    inverted = !inverted;
                    node = graph.resolve(graph.input(node, 0));
                    continue;
                }
                if coverable && graph.op(node) == Opcode::Compare {
                    let lhs = graph.resolve(graph.input(node, 0));
                    let rhs = graph.resolve(graph.input(node, 1));
                    // Floating-point and pointer comparisons keep their old path.
                    let scalar = graph.rust_type(lhs).is_some_and(|ty| {
                        matches!(
                            graph.oracle.shape(ty),
                            TypeShape::Int { .. } | TypeShape::Bool | TypeShape::Char
                        )
                    });
                    if scalar {
                        covered.push(node);
                        let condition =
                            condition(ComparePredicate::from_payload(graph.payload(node)));
                        result.direct.insert(
                            branch,
                            DirectBranch {
                                lhs,
                                rhs: Some(rhs),
                                condition: if inverted {
                                    inverse(condition)
                                } else {
                                    condition
                                },
                            },
                        );
                        result.covered.extend(covered);
                        break;
                    }
                }
                if boolean && !covered.is_empty() {
                    result.direct.insert(
                        branch,
                        DirectBranch {
                            lhs: node,
                            rhs: None,
                            condition: if inverted {
                                Condition::Eq
                            } else {
                                Condition::NotEq
                            },
                        },
                    );
                    result.covered.extend(covered);
                }
                break;
            }
        }
        result
    }

    pub(super) fn covers(&self, node: NodeId) -> bool {
        self.covered.contains(&node)
    }

    pub(super) fn get(&self, branch: NodeId) -> Option<DirectBranch> {
        self.direct.get(&branch).copied()
    }
}

pub(super) fn condition(predicate: ComparePredicate) -> Condition {
    match predicate {
        ComparePredicate::Equal => Condition::Eq,
        ComparePredicate::NotEqual => Condition::NotEq,
        ComparePredicate::SignedLess => Condition::SignedLess,
        ComparePredicate::SignedLessEqual => Condition::SignedLessEq,
        ComparePredicate::SignedGreater => Condition::SignedGreater,
        ComparePredicate::SignedGreaterEqual => Condition::SignedGreaterEq,
        ComparePredicate::UnsignedLess => Condition::UnsignedBelow,
        ComparePredicate::UnsignedLessEqual => Condition::UnsignedBelowEq,
        ComparePredicate::UnsignedGreater => Condition::UnsignedAbove,
        ComparePredicate::UnsignedGreaterEqual => Condition::UnsignedAboveEq,
    }
}

pub(super) fn inverse(condition: Condition) -> Condition {
    match condition {
        Condition::Eq => Condition::NotEq,
        Condition::NotEq => Condition::Eq,
        Condition::SignedLess => Condition::SignedGreaterEq,
        Condition::SignedLessEq => Condition::SignedGreater,
        Condition::SignedGreater => Condition::SignedLessEq,
        Condition::SignedGreaterEq => Condition::SignedLess,
        Condition::UnsignedBelow => Condition::UnsignedAboveEq,
        Condition::UnsignedBelowEq => Condition::UnsignedAbove,
        Condition::UnsignedAbove => Condition::UnsignedBelowEq,
        Condition::UnsignedAboveEq => Condition::UnsignedBelow,
    }
}
