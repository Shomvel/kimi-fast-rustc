//! Reference interpreter for R-level graphs (`docs/design/ir/semantics.md`).
//!
//! Fixed nodes run in control order; pure nodes are evaluated on demand;
//! all `Phi`s of a `Region` take their values at once when control enters
//! it. It locates miscompiles (run a graph before and after a pass on the
//! same input); correctness is still judged by real execution.

use std::collections::HashMap;

use kfrc_body::{IntegerWidth, TyId, TypeOracle, TypeShape};

use crate::graph::{ComparePredicate, FunctionGraph, NodeId, Opcode, ValueType};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The returned value's bits (zero-extended), or 0 for ignored returns.
    Return(u64),
    /// Execution reached `Unreachable` (undefined behaviour or divergence).
    Unreachable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InterpretError {
    /// Undefined behaviour the interpreter can detect.
    UndefinedBehaviour(String),
    /// A value of `Undefined` (poison) reached a point that observes it.
    PoisonObserved(NodeId),
    UnknownCallee(String),
    OutOfFuel,
    Malformed(String),
}

/// Resolves calls to other functions by symbol.
pub trait CallHandler {
    fn call(&mut self, symbol: &str, arguments: &[u64]) -> Result<u64, InterpretError>;
}

impl<F: FnMut(&str, &[u64]) -> Result<u64, InterpretError>> CallHandler for F {
    fn call(&mut self, symbol: &str, arguments: &[u64]) -> Result<u64, InterpretError> {
        self(symbol, arguments)
    }
}

pub struct Interpreter<'a> {
    graph: &'a FunctionGraph,
    control_users: HashMap<NodeId, Vec<NodeId>>,
    region_of_jump: HashMap<NodeId, (NodeId, usize)>,
    phis_of_region: HashMap<NodeId, Vec<NodeId>>,
    pub fuel: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Scalar {
    bits: u64,
    poison: bool,
}

impl Scalar {
    fn new(bits: u64) -> Self {
        Self { bits, poison: false }
    }
}

struct Frame {
    phi_values: HashMap<NodeId, Scalar>,
    fixed_values: HashMap<NodeId, Scalar>,
    cache: HashMap<NodeId, Scalar>,
    arguments: Vec<u64>,
}

impl<'a> Interpreter<'a> {
    pub fn new(graph: &'a FunctionGraph) -> Self {
        let mut control_users: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        let mut region_of_jump = HashMap::new();
        let mut phis_of_region: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for node in graph.ids() {
            let control = graph.control(node);
            if !control.is_none() {
                control_users.entry(control).or_default().push(node);
            }
            match graph.op(node) {
                Opcode::Region => {
                    for (slot, jump) in graph.inputs(node).iter().enumerate() {
                        region_of_jump.insert(*jump, (node, slot));
                    }
                }
                Opcode::Phi => phis_of_region.entry(graph.input(node, 0)).or_default().push(node),
                _ => {}
            }
        }
        Self {
            graph,
            control_users,
            region_of_jump,
            phis_of_region,
            fuel: 10_000_000,
        }
    }

    pub fn run(&mut self, arguments: &[u64], calls: &mut dyn CallHandler) -> Result<Outcome, InterpretError> {
        let graph = self.graph;
        let mut frame = Frame {
            phi_values: HashMap::new(),
            fixed_values: HashMap::new(),
            cache: HashMap::new(),
            arguments: arguments.to_vec(),
        };
        let mut control = graph
            .find_projection(graph.start, 0)
            .ok_or_else(|| InterpretError::Malformed("start has no control projection".to_owned()))?;
        loop {
            if self.fuel == 0 {
                return Err(InterpretError::OutOfFuel);
            }
            self.fuel -= 1;
            let users = self.control_users.get(&control).cloned().unwrap_or_default();
            // Fixed value nodes (division) evaluate when control passes them.
            for user in &users {
                if matches!(
                    graph.op(*user),
                    Opcode::DivSigned | Opcode::DivUnsigned | Opcode::RemSigned | Opcode::RemUnsigned
                ) {
                    let value = self.evaluate_pure(*user, &mut frame, true)?;
                    frame.fixed_values.insert(*user, value);
                }
            }
            let next = users.iter().copied().find(|user| {
                matches!(
                    graph.op(*user),
                    Opcode::If | Opcode::Jump | Opcode::Return | Opcode::Unreachable | Opcode::Call
                )
            });
            let Some(next) = next else {
                return Err(InterpretError::Malformed(format!("control {control} has no successor")));
            };
            match graph.op(next) {
                Opcode::If => {
                    let condition = self.evaluate(graph.input(next, 0), &mut frame)?;
                    if condition.poison {
                        return Err(InterpretError::PoisonObserved(next));
                    }
                    let index = if condition.bits & 1 == 1 { 0 } else { 1 };
                    control = graph
                        .find_projection(next, index)
                        .ok_or_else(|| InterpretError::Malformed(format!("{next} lacks projection {index}")))?;
                }
                Opcode::Jump => {
                    let (region, slot) = *self
                        .region_of_jump
                        .get(&next)
                        .ok_or_else(|| InterpretError::Malformed(format!("{next} jumps nowhere")))?;
                    let phis = self.phis_of_region.get(&region).cloned().unwrap_or_default();
                    let mut values = Vec::with_capacity(phis.len());
                    for phi in &phis {
                        let operand = graph.input(*phi, slot + 1);
                        let value = if matches!(graph.value_type(*phi), ValueType::Effect) {
                            Scalar::new(0)
                        } else {
                            self.evaluate(operand, &mut frame)?
                        };
                        values.push(value);
                    }
                    for (phi, value) in phis.iter().zip(values) {
                        frame.phi_values.insert(*phi, value);
                    }
                    frame.cache.clear();
                    control = region;
                }
                Opcode::Return => {
                    let inputs = graph.inputs(next);
                    if inputs.len() == 1 {
                        return Ok(Outcome::Return(0));
                    }
                    let value = self.evaluate(inputs[1], &mut frame)?;
                    if value.poison {
                        return Err(InterpretError::PoisonObserved(next));
                    }
                    return Ok(Outcome::Return(value.bits));
                }
                Opcode::Unreachable => return Ok(Outcome::Unreachable),
                Opcode::Call => {
                    let callee = &graph.callees[graph.payload(next) as usize];
                    let mut arguments = Vec::new();
                    for (input, mode) in graph.inputs(next)[1..].iter().zip(&callee.abi.args) {
                        if *mode == kfrc_body::PassMode::Ignore {
                            continue;
                        }
                        let value = self.evaluate(*input, &mut frame)?;
                        if value.poison {
                            return Err(InterpretError::PoisonObserved(next));
                        }
                        arguments.push(value.bits);
                    }
                    let result = calls.call(&callee.symbol, &arguments)?;
                    if let Some(value) = graph.find_projection(next, 2) {
                        let ty = graph.rust_type(value).expect("call value is a Rust type");
                        frame.fixed_values.insert(value, Scalar::new(self.normalize(ty, result)));
                    }
                    frame.cache.clear();
                    control = graph
                        .find_projection(next, 0)
                        .ok_or_else(|| InterpretError::Malformed(format!("{next} lacks a control projection")))?;
                }
                _ => unreachable!("filtered above"),
            }
        }
    }

    fn evaluate(&self, node: NodeId, frame: &mut Frame) -> Result<Scalar, InterpretError> {
        self.evaluate_pure(node, frame, false)
    }

    fn evaluate_pure(&self, node: NodeId, frame: &mut Frame, executing_fixed: bool) -> Result<Scalar, InterpretError> {
        let graph = self.graph;
        if let Some(value) = frame.phi_values.get(&node).or_else(|| frame.fixed_values.get(&node))
            && !executing_fixed
        {
            return Ok(*value);
        }
        if let Some(value) = frame.cache.get(&node) {
            return Ok(*value);
        }
        let op = graph.op(node);
        let ty = graph.rust_type(node);
        let value = match op {
            Opcode::Const => Scalar::new(graph.constant_value(node) as u64),
            Opcode::Undefined => Scalar { bits: 0, poison: true },
            Opcode::Projection => {
                let source = graph.input(node, 0);
                if graph.op(source) != Opcode::Start {
                    return Err(InterpretError::Malformed(format!("value {node} read before its call ran")));
                }
                let index = graph.payload(node) as usize - 2;
                let ty = ty.expect("parameter has a Rust type");
                let passed = graph.abi.args[..index].iter().filter(|mode| **mode != kfrc_body::PassMode::Ignore).count();
                let bits = if graph.abi.args[index] == kfrc_body::PassMode::Ignore {
                    0
                } else {
                    *frame
                        .arguments
                        .get(passed)
                        .ok_or_else(|| InterpretError::Malformed("too few arguments".to_owned()))?
                };
                Scalar::new(self.normalize(ty, bits))
            }
            Opcode::Phi => {
                return Err(InterpretError::Malformed(format!("phi {node} read before its region ran")));
            }
            Opcode::DivSigned | Opcode::DivUnsigned | Opcode::RemSigned | Opcode::RemUnsigned if !executing_fixed => {
                return Err(InterpretError::Malformed(format!("{node} read before control reached it")));
            }
            _ => {
                let inputs = graph.inputs(node).to_vec();
                let mut values = Vec::with_capacity(inputs.len());
                for input in &inputs {
                    values.push(self.evaluate(*input, frame)?);
                }
                let ty = ty.ok_or_else(|| InterpretError::Malformed(format!("{node} has no value type")))?;
                self.apply(node, op, ty, &inputs, &values)?
            }
        };
        if !executing_fixed {
            frame.cache.insert(node, value);
        }
        Ok(value)
    }

    fn width_and_sign(&self, ty: TyId) -> (u32, bool) {
        match self.graph.oracle.shape(ty) {
            TypeShape::Bool => (1, false),
            TypeShape::Char => (32, false),
            TypeShape::Int { width, signed } => (u32::from(width_bits(*width)), *signed),
            _ => (64, false),
        }
    }

    /// Canonical bits for a value of `ty`: zero-extended.
    fn normalize(&self, ty: TyId, bits: u64) -> u64 {
        let (width, _) = self.width_and_sign(ty);
        if width >= 64 { bits } else { bits & ((1u64 << width) - 1) }
    }

    fn signed(&self, ty: TyId, bits: u64) -> i64 {
        let (width, _) = self.width_and_sign(ty);
        if width >= 64 {
            bits as i64
        } else {
            let shift = 64 - width;
            ((bits << shift) as i64) >> shift
        }
    }

    fn apply(&self, node: NodeId, op: Opcode, ty: TyId, inputs: &[NodeId], values: &[Scalar]) -> Result<Scalar, InterpretError> {
        let poison = values.iter().any(|value| value.poison);
        let result_ty = ty;
        let operand_ty = inputs.first().and_then(|input| self.graph.rust_type(*input)).unwrap_or(ty);
        let (width, _) = self.width_and_sign(operand_ty);
        let a = values.first().map_or(0, |value| value.bits);
        let b = values.get(1).map_or(0, |value| value.bits);
        let sa = self.signed(operand_ty, a);
        let sb = self.signed(operand_ty, b);
        let shift_amount = |amount: u64| (amount % u64::from(width.max(1))) as u32;
        let bits = match op {
            Opcode::Add => a.wrapping_add(b),
            Opcode::Sub => a.wrapping_sub(b),
            Opcode::Mul => a.wrapping_mul(b),
            Opcode::DivSigned | Opcode::RemSigned | Opcode::DivUnsigned | Opcode::RemUnsigned => {
                if poison {
                    return Err(InterpretError::PoisonObserved(node));
                }
                if b == 0 || (matches!(op, Opcode::DivSigned | Opcode::RemSigned) && sb == -1 && sa == self.signed(operand_ty, 1u64 << (width - 1))) {
                    return Err(InterpretError::UndefinedBehaviour(format!("{node}: division by zero or overflow")));
                }
                match op {
                    Opcode::DivSigned => sa.wrapping_div(sb) as u64,
                    Opcode::RemSigned => sa.wrapping_rem(sb) as u64,
                    Opcode::DivUnsigned => a / b,
                    _ => a % b,
                }
            }
            Opcode::BitAnd => a & b,
            Opcode::BitOr => a | b,
            Opcode::BitXor => a ^ b,
            Opcode::ShiftLeft => a.wrapping_shl(shift_amount(b)),
            Opcode::ShiftRightArithmetic => (sa >> shift_amount(b)) as u64,
            Opcode::ShiftRightLogical => a.wrapping_shr(shift_amount(b)),
            Opcode::Negate => a.wrapping_neg(),
            Opcode::BitNot => !a,
            Opcode::IntCast => {
                // Extend from the source type, then truncate to the target.
                let (_, source_signed) = self.width_and_sign(operand_ty);
                if source_signed { sa as u64 } else { a }
            }
            Opcode::Compare => {
                let outcome = match ComparePredicate::from_payload(self.graph.payload(node)) {
                    ComparePredicate::Equal => a == b,
                    ComparePredicate::NotEqual => a != b,
                    ComparePredicate::SignedLess => sa < sb,
                    ComparePredicate::SignedLessEqual => sa <= sb,
                    ComparePredicate::SignedGreater => sa > sb,
                    ComparePredicate::SignedGreaterEqual => sa >= sb,
                    ComparePredicate::UnsignedLess => a < b,
                    ComparePredicate::UnsignedLessEqual => a <= b,
                    ComparePredicate::UnsignedGreater => a > b,
                    ComparePredicate::UnsignedGreaterEqual => a >= b,
                };
                u64::from(outcome)
            }
            other => return Err(InterpretError::Malformed(format!("{node}: {other:?} is not a pure value"))),
        };
        Ok(Scalar {
            bits: self.normalize(result_ty, bits),
            poison,
        })
    }
}

fn width_bits(width: IntegerWidth) -> u8 {
    width.bits(64)
}
