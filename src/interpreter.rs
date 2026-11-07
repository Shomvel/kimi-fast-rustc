//! Single-basic-block i64 reference evaluation for the sea-of-nodes graph.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::son::{BinaryOp, CompareOp, Graph, NodeId, NodeKind, UnaryOp, ValidationError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsupportedReason {
    NodeKind(NodeKind),
    MultipleStarts,
    MultipleReturns,
    ReturnWithoutValue,
    CodeAfterReturn,
    InvalidControlFlow,
    MemoryInputs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrapKind {
    DivisionByZero,
    RemainderByZero,
    DivisionOverflow,
    RemainderOverflow,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InterpretError {
    GraphValidation(ValidationError),
    MissingReturn,
    MissingParameter {
        node: NodeId,
        index: usize,
        available: usize,
    },
    Unsupported {
        node: NodeId,
        reason: UnsupportedReason,
    },
    InvalidValueReference {
        node: NodeId,
        input: NodeId,
    },
    Trap {
        node: NodeId,
        kind: TrapKind,
    },
}

impl fmt::Display for InterpretError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GraphValidation(error) => write!(formatter, "invalid graph: {error}"),
            Self::MissingReturn => formatter.write_str("graph has no return"),
            Self::MissingParameter {
                node,
                index,
                available,
            } => write!(
                formatter,
                "parameter {index} at {node} is missing; only {available} arguments were provided"
            ),
            Self::Unsupported { node, reason } => match reason {
                UnsupportedReason::NodeKind(kind) => {
                    write!(formatter, "unsupported node {node}: {kind:?}")
                }
                UnsupportedReason::MultipleStarts => {
                    write!(formatter, "unsupported node {node}: multiple start nodes")
                }
                UnsupportedReason::MultipleReturns => {
                    write!(formatter, "unsupported node {node}: multiple returns")
                }
                UnsupportedReason::ReturnWithoutValue => {
                    write!(
                        formatter,
                        "unsupported node {node}: return without an i64 value"
                    )
                }
                UnsupportedReason::CodeAfterReturn => {
                    write!(formatter, "unsupported node {node}: code after return")
                }
                UnsupportedReason::InvalidControlFlow => write!(
                    formatter,
                    "unsupported node {node}: return control is not the single start node"
                ),
                UnsupportedReason::MemoryInputs => {
                    write!(formatter, "unsupported node {node}: memory-state inputs")
                }
            },
            Self::InvalidValueReference { node, input } => write!(
                formatter,
                "node {node} reads {input}, which is not a previously evaluated value"
            ),
            Self::Trap { node, kind } => {
                let message = match kind {
                    TrapKind::DivisionByZero => "division by zero",
                    TrapKind::RemainderByZero => "remainder by zero",
                    TrapKind::DivisionOverflow => "division overflow (i64::MIN / -1)",
                    TrapKind::RemainderOverflow => "remainder overflow (i64::MIN % -1)",
                };
                write!(formatter, "arithmetic trap at {node}: {message}")
            }
        }
    }
}

impl Error for InterpretError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::GraphValidation(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ValidationError> for InterpretError {
    fn from(error: ValidationError) -> Self {
        Self::GraphValidation(error)
    }
}

pub fn interpret(graph: &Graph, arguments: &[i64]) -> Result<i64, InterpretError> {
    graph.validate()?;
    validate_supported(graph)?;

    let mut values = BTreeMap::new();
    for (id, node) in graph.iter() {
        let value = match node.kind {
            NodeKind::Start => continue,
            NodeKind::Const(value) => value,
            NodeKind::Parameter { index } => {
                *arguments
                    .get(index)
                    .ok_or(InterpretError::MissingParameter {
                        node: id,
                        index,
                        available: arguments.len(),
                    })?
            }
            NodeKind::Unary(operation) => {
                let input = value_of(id, node.data[0], &values)?;
                match operation {
                    UnaryOp::Neg => input.wrapping_neg(),
                    UnaryOp::Not => !input,
                }
            }
            NodeKind::Binary(operation) => {
                let left = value_of(id, node.data[0], &values)?;
                let right = value_of(id, node.data[1], &values)?;
                evaluate_binary(id, operation, left, right)?
            }
            NodeKind::Compare(operation) => {
                let left = value_of(id, node.data[0], &values)?;
                let right = value_of(id, node.data[1], &values)?;
                i64::from(match operation {
                    CompareOp::Eq => left == right,
                    CompareOp::Ne => left != right,
                    CompareOp::Lt => left < right,
                    CompareOp::Le => left <= right,
                    CompareOp::Gt => left > right,
                    CompareOp::Ge => left >= right,
                })
            }
            NodeKind::Phi => unreachable!("unsupported Phi nodes are rejected before evaluation"),
            NodeKind::Return => return value_of(id, node.data[0], &values),
        };
        values.insert(id, value);
    }

    unreachable!("a supported graph always returns during evaluation")
}

fn validate_supported(graph: &Graph) -> Result<(), InterpretError> {
    let mut returns = graph
        .iter()
        .filter(|(_, node)| node.kind == NodeKind::Return)
        .map(|(id, _)| id);
    if returns.next().is_none() {
        return Err(InterpretError::MissingReturn);
    }
    if let Some(second) = returns.next() {
        return Err(unsupported(second, UnsupportedReason::MultipleReturns));
    }

    let mut starts = graph
        .iter()
        .filter(|(_, node)| node.kind == NodeKind::Start)
        .map(|(id, _)| id);
    starts.next();
    if let Some(second) = starts.next() {
        return Err(unsupported(second, UnsupportedReason::MultipleStarts));
    }

    let mut returned = false;
    for (id, node) in graph.iter() {
        if !node.memory.is_empty() {
            return Err(unsupported(id, UnsupportedReason::MemoryInputs));
        }
        if returned {
            return Err(unsupported(id, UnsupportedReason::CodeAfterReturn));
        }
        match node.kind {
            NodeKind::Phi => {
                return Err(unsupported(id, UnsupportedReason::NodeKind(NodeKind::Phi)));
            }
            NodeKind::Return => {
                if node.data.is_empty() {
                    return Err(unsupported(id, UnsupportedReason::ReturnWithoutValue));
                }
                let control = node.control[0];
                if graph.node(control).map(|node| node.kind) != Some(NodeKind::Start) {
                    return Err(unsupported(id, UnsupportedReason::InvalidControlFlow));
                }
                returned = true;
            }
            _ => {}
        }
    }
    Ok(())
}

fn evaluate_binary(
    node: NodeId,
    operation: BinaryOp,
    left: i64,
    right: i64,
) -> Result<i64, InterpretError> {
    let value = match operation {
        BinaryOp::Add => left.wrapping_add(right),
        BinaryOp::Sub => left.wrapping_sub(right),
        BinaryOp::Mul => left.wrapping_mul(right),
        BinaryOp::Div => {
            if right == 0 {
                return Err(trap(node, TrapKind::DivisionByZero));
            }
            if left == i64::MIN && right == -1 {
                return Err(trap(node, TrapKind::DivisionOverflow));
            }
            left / right
        }
        BinaryOp::Rem => {
            if right == 0 {
                return Err(trap(node, TrapKind::RemainderByZero));
            }
            if left == i64::MIN && right == -1 {
                return Err(trap(node, TrapKind::RemainderOverflow));
            }
            left % right
        }
        BinaryOp::And => left & right,
        BinaryOp::Or => left | right,
        BinaryOp::Xor => left ^ right,
        BinaryOp::Shl => left.wrapping_shl(right as u32),
        BinaryOp::Shr => left.wrapping_shr(right as u32),
    };
    Ok(value)
}

fn value_of(
    node: NodeId,
    input: NodeId,
    values: &BTreeMap<NodeId, i64>,
) -> Result<i64, InterpretError> {
    values
        .get(&input)
        .copied()
        .ok_or(InterpretError::InvalidValueReference { node, input })
}

fn unsupported(node: NodeId, reason: UnsupportedReason) -> InterpretError {
    InterpretError::Unsupported { node, reason }
}

fn trap(node: NodeId, kind: TrapKind) -> InterpretError {
    InterpretError::Trap { node, kind }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unary(operation: UnaryOp, input: i64) -> Result<i64, InterpretError> {
        let mut graph = Graph::new();
        let start = graph.start();
        let input = graph.constant(input);
        let value = graph.unary(operation, input);
        graph.return_(start, Some(value));
        interpret(&graph, &[])
    }

    fn binary(operation: BinaryOp, left: i64, right: i64) -> Result<i64, InterpretError> {
        let mut graph = Graph::new();
        let start = graph.start();
        let left = graph.constant(left);
        let right = graph.constant(right);
        let value = graph.binary(operation, left, right);
        graph.return_(start, Some(value));
        interpret(&graph, &[])
    }

    #[test]
    fn evaluates_constants_parameters_unary_and_binary_nodes() {
        let mut graph = Graph::new();
        let start = graph.start();
        let first = graph.parameter(0);
        let second = graph.parameter(1);
        let repeated = graph.parameter(0);
        let sum = graph.binary(BinaryOp::Add, first, second);
        let product = graph.binary(BinaryOp::Mul, sum, repeated);
        let negated = graph.unary(UnaryOp::Neg, product);
        graph.return_(start, Some(negated));

        assert_eq!(interpret(&graph, &[7, 5]), Ok(-84));
        assert_eq!(
            interpret(&graph, &[7]),
            Err(InterpretError::MissingParameter {
                node: second,
                index: 1,
                available: 1,
            })
        );
    }

    #[test]
    fn wraps_negation_addition_subtraction_and_multiplication() {
        assert_eq!(unary(UnaryOp::Neg, i64::MIN), Ok(i64::MIN));
        assert_eq!(unary(UnaryOp::Not, i64::MIN), Ok(i64::MAX));
        assert_eq!(binary(BinaryOp::Add, i64::MAX, 1), Ok(i64::MIN));
        assert_eq!(binary(BinaryOp::Sub, i64::MIN, 1), Ok(i64::MAX));
        assert_eq!(binary(BinaryOp::Mul, i64::MAX, 2), Ok(-2));
        assert_eq!(binary(BinaryOp::Mul, i64::MIN, -1), Ok(i64::MIN));
    }

    #[test]
    fn evaluates_division_remainder_and_bitwise_operations() {
        let cases = [
            (BinaryOp::Div, -42, 4, -10),
            (BinaryOp::Rem, -43, 4, -3),
            (BinaryOp::And, 0b1100, 0b1010, 0b1000),
            (BinaryOp::Or, 0b1100, 0b1010, 0b1110),
            (BinaryOp::Xor, 0b1100, 0b1010, 0b0110),
        ];
        for (operation, left, right, expected) in cases {
            assert_eq!(
                binary(operation, left, right),
                Ok(expected),
                "{operation:?}"
            );
        }
    }

    #[test]
    fn masks_shift_counts_and_preserves_arithmetic_right_shift() {
        let cases = [
            (BinaryOp::Shl, 1, 64, 1),
            (BinaryOp::Shl, 1, 65, 2),
            (BinaryOp::Shl, 1, -1, i64::MIN),
            (BinaryOp::Shl, i64::MAX, 1, -2),
            (BinaryOp::Shr, -8, 1, -4),
            (BinaryOp::Shr, i64::MIN, 63, -1),
            (BinaryOp::Shr, -8, 64, -8),
        ];
        for (operation, left, right, expected) in cases {
            assert_eq!(
                binary(operation, left, right),
                Ok(expected),
                "{operation:?}"
            );
        }
    }

    #[test]
    fn signed_comparisons_produce_zero_or_one() {
        let cases = [
            (CompareOp::Eq, [0, 1, 0]),
            (CompareOp::Ne, [1, 0, 1]),
            (CompareOp::Lt, [1, 0, 0]),
            (CompareOp::Le, [1, 1, 0]),
            (CompareOp::Gt, [0, 0, 1]),
            (CompareOp::Ge, [0, 1, 1]),
        ];
        for (operation, expected) in cases {
            for ((left, right), expected) in [(4, 5), (5, 5), (6, 5)].into_iter().zip(expected) {
                let mut graph = Graph::new();
                let start = graph.start();
                let left = graph.constant(left);
                let right = graph.constant(right);
                let comparison = graph.compare(operation, left, right);
                graph.return_(start, Some(comparison));
                assert_eq!(interpret(&graph, &[]), Ok(expected), "{operation:?}");
            }
        }

        let mut graph = Graph::new();
        let start = graph.start();
        let minimum = graph.constant(i64::MIN);
        let zero = graph.constant(0);
        let comparison = graph.compare(CompareOp::Lt, minimum, zero);
        graph.return_(start, Some(comparison));
        assert_eq!(interpret(&graph, &[]), Ok(1));
    }

    #[test]
    fn reports_division_and_remainder_traps() {
        let cases = [
            (BinaryOp::Div, 1, 0, TrapKind::DivisionByZero),
            (BinaryOp::Rem, 1, 0, TrapKind::RemainderByZero),
            (BinaryOp::Div, i64::MIN, -1, TrapKind::DivisionOverflow),
            (BinaryOp::Rem, i64::MIN, -1, TrapKind::RemainderOverflow),
        ];
        for (operation, left, right, kind) in cases {
            let mut graph = Graph::new();
            let start = graph.start();
            let left = graph.constant(left);
            let right = graph.constant(right);
            let value = graph.binary(operation, left, right);
            graph.return_(start, Some(value));
            assert_eq!(
                interpret(&graph, &[]),
                Err(InterpretError::Trap { node: value, kind })
            );
            assert!(
                interpret(&graph, &[])
                    .unwrap_err()
                    .to_string()
                    .contains("trap")
            );
        }
    }

    #[test]
    fn traps_in_nodes_executed_before_return_even_if_value_is_unused() {
        let mut graph = Graph::new();
        let start = graph.start();
        let one = graph.constant(1);
        let zero = graph.constant(0);
        let division = graph.binary(BinaryOp::Div, one, zero);
        graph.return_(start, Some(one));

        assert_eq!(
            interpret(&graph, &[]),
            Err(InterpretError::Trap {
                node: division,
                kind: TrapKind::DivisionByZero,
            })
        );
    }

    #[test]
    fn rejects_phi_and_multiple_control_flow_explicitly() {
        let mut graph = Graph::new();
        let start = graph.start();
        let one = graph.constant(1);
        let phi = graph.phi(vec![start], vec![one]);
        graph.return_(start, Some(phi));
        assert_eq!(
            interpret(&graph, &[]),
            Err(InterpretError::Unsupported {
                node: phi,
                reason: UnsupportedReason::NodeKind(NodeKind::Phi),
            })
        );

        let mut graph = Graph::new();
        graph.start();
        let second = graph.start();
        let one = graph.constant(1);
        graph.return_(second, Some(one));
        assert_eq!(
            interpret(&graph, &[]),
            Err(InterpretError::Unsupported {
                node: second,
                reason: UnsupportedReason::MultipleStarts,
            })
        );

        let mut graph = Graph::new();
        let start = graph.start();
        let one = graph.constant(1);
        graph.return_(start, Some(one));
        let second = graph.return_(start, Some(one));
        assert_eq!(
            interpret(&graph, &[]),
            Err(InterpretError::Unsupported {
                node: second,
                reason: UnsupportedReason::MultipleReturns,
            })
        );
    }

    #[test]
    fn rejects_missing_empty_and_invalid_returns() {
        let mut graph = Graph::new();
        graph.constant(1);
        assert_eq!(interpret(&graph, &[]), Err(InterpretError::MissingReturn));

        let mut graph = Graph::new();
        let start = graph.start();
        let empty = graph.return_(start, None);
        assert_eq!(
            interpret(&graph, &[]),
            Err(InterpretError::Unsupported {
                node: empty,
                reason: UnsupportedReason::ReturnWithoutValue,
            })
        );

        let mut graph = Graph::new();
        graph.start();
        let one = graph.constant(1);
        let result = graph.return_(one, Some(one));
        assert_eq!(
            interpret(&graph, &[]),
            Err(InterpretError::Unsupported {
                node: result,
                reason: UnsupportedReason::InvalidControlFlow,
            })
        );

        let mut graph = Graph::new();
        let start = graph.start();
        let one = graph.constant(1);
        graph.return_(start, Some(one));
        let after = graph.constant(2);
        assert_eq!(
            interpret(&graph, &[]),
            Err(InterpretError::Unsupported {
                node: after,
                reason: UnsupportedReason::CodeAfterReturn,
            })
        );
    }

    #[test]
    fn rejects_memory_inputs_and_early_value_references() {
        let mut graph = Graph::new();
        let start = graph.start();
        let one = graph.constant(1);
        let result =
            graph.add_node_with_memory(NodeKind::Return, vec![start], vec![one], vec![one]);
        assert_eq!(
            interpret(&graph, &[]),
            Err(InterpretError::Unsupported {
                node: result,
                reason: UnsupportedReason::MemoryInputs,
            })
        );

        let mut graph = Graph::new();
        graph.start();
        let addition = graph.add_node(
            NodeKind::Binary(BinaryOp::Add),
            Vec::new(),
            vec![NodeId(2), NodeId(2)],
        );
        graph.constant(1);
        let start = NodeId(0);
        graph.return_(start, Some(addition));
        assert_eq!(
            interpret(&graph, &[]),
            Err(InterpretError::InvalidValueReference {
                node: addition,
                input: NodeId(2),
            })
        );
    }
}
