//! Independent R execution with lazy answers. This executes the original graph;
//! it does not call specialization or manufacture an S graph for comparison.
//! Dynamic storage, representation reinterpretation and indirect calls are
//! deliberately outside this first R interpreter subset.
use std::collections::HashMap;
use std::sync::Arc;

use super::{
    InterpretError, Interpreter, Scalar, SemanticCallHandler, SemanticMemory, SemanticOutcome,
    Value,
};
use crate::analysis::cfg::is_anchored_value;
use crate::specialize::{HoleAnswer, SpecializationOracle};
use crate::{GraphLevel, Hole, NodeId, Opcode, ValueType};
use kfrc_body::{RecordedTypes, TyId, TypeShape};

type Result<T> = std::result::Result<T, InterpretError>;
fn malformed(detail: impl Into<String>) -> InterpretError {
    InterpretError::Malformed(detail.into())
}

impl Interpreter<'_> {
    /// Execute a verified definition with per-execution answers. The provider
    /// is queried only when control/value demand reaches the corresponding hole.
    pub fn run_rust_values(
        &mut self,
        arguments: &[Value],
        calls: &mut dyn SemanticCallHandler,
        oracle: &mut dyn SpecializationOracle,
    ) -> Result<SemanticOutcome> {
        if self.graph.level != GraphLevel::Rust {
            return Err(malformed("run_rust_values requires an R graph"));
        }
        crate::verify::check(self.graph).map_err(|error| malformed(error.to_string()))?;
        let fuel = self.fuel;
        let mut runner = Runner {
            interpreter: self,
            oracle,
            calls,
            arguments,
            runtime_types: RecordedTypes::new(),
            types: HashMap::new(),
            answers: HashMap::new(),
            phis: HashMap::new(),
            fixed: HashMap::new(),
            cache: HashMap::new(),
            memory: SemanticMemory::new(),
            active: NodeId::NONE,
            fuel,
        };
        let result = runner.run();
        self.fuel = runner.fuel;
        result
    }
}

struct Runner<'i, 'g, 'a> {
    interpreter: &'i Interpreter<'g>,
    oracle: &'a mut dyn SpecializationOracle,
    calls: &'a mut dyn SemanticCallHandler,
    arguments: &'a [Value],
    runtime_types: RecordedTypes,
    types: HashMap<TyId, TyId>,
    answers: HashMap<u32, HoleAnswer>,
    phis: HashMap<NodeId, Value>,
    fixed: HashMap<NodeId, Value>,
    cache: HashMap<NodeId, Value>,
    memory: SemanticMemory,
    active: NodeId,
    fuel: u64,
}

impl Runner<'_, '_, '_> {
    fn runtime_type(&mut self, source: TyId) -> Result<TyId> {
        if let Some(ty) = self.types.get(&source) {
            return Ok(*ty);
        }
        let ty = self
            .oracle
            .instantiate_type(source, &mut self.runtime_types)
            .map_err(malformed)?;
        self.types.insert(source, ty);
        Ok(ty)
    }
    fn check_value(&mut self, source: TyId, value: &Value) -> Result<()> {
        let ty = self.runtime_type(source)?;
        super::value::check(&self.runtime_types, ty, value)
    }
    fn poison(&mut self, source: TyId) -> Result<Value> {
        let ty = self.runtime_type(source)?;
        super::value::poison(&self.runtime_types, ty)
    }
    fn answer(&mut self, index: u32) -> Result<HoleAnswer> {
        if let Some(answer) = self.answers.get(&index) {
            return Ok(answer.clone());
        }
        let hole = &self.interpreter.graph.holes[index as usize];
        let answer = self.oracle.answer(hole).map_err(malformed)?;
        let correct = matches!(
            (hole, &answer),
            (Hole::SizeAlign(_), HoleAnswer::SizeAlign(_))
                | (Hole::NeedsDrop(_), HoleAnswer::NeedsDrop(_))
                | (Hole::Resolve { .. }, HoleAnswer::Resolved { .. })
                | (Hole::VariantEncoding(_), HoleAnswer::VariantEncoding(_))
        );
        if !correct {
            return Err(malformed("answer kind differs from requested R hole"));
        }
        if let HoleAnswer::SizeAlign(layout) = &answer
            && !layout.align.is_power_of_two()
        {
            return Err(malformed("SizeAlign answer has invalid alignment"));
        }
        self.answers.insert(index, answer.clone());
        Ok(answer)
    }
    fn run(&mut self) -> Result<SemanticOutcome> {
        let graph = self.interpreter.graph;
        let signature = graph.rust_signature.as_ref().expect("verified R signature");
        if self.arguments.len() != signature.args.len() {
            return Err(malformed("R argument count differs from source signature"));
        }
        for (&ty, value) in signature.args.iter().zip(self.arguments) {
            self.check_value(ty, value)?;
            if value.contains_poison() {
                return Err(InterpretError::PoisonObserved(graph.start));
            }
        }
        let mut control = graph
            .find_projection(graph.start, 0)
            .ok_or_else(|| malformed("R Start lacks control"))?;
        loop {
            if self.fuel == 0 {
                return Err(InterpretError::OutOfFuel);
            }
            self.fuel -= 1;
            self.active = control;
            let users = self
                .interpreter
                .control_users
                .get(&control)
                .cloned()
                .unwrap_or_default();
            for user in &users {
                if is_anchored_value(graph.op(*user)) {
                    self.fixed.remove(user);
                }
            }
            for user in &users {
                if is_anchored_value(graph.op(*user)) {
                    self.value(*user)?;
                }
            }
            let next = users
                .into_iter()
                .find(|node| graph.op(*node).is_fixed() && !is_anchored_value(graph.op(*node)))
                .ok_or_else(|| malformed("R control has no successor"))?;
            match graph.op(next) {
                Opcode::If => {
                    let condition = self.scalar(graph.input(next, 0), next)?;
                    control = graph
                        .find_projection(next, u32::from(condition & 1 == 0))
                        .ok_or_else(|| malformed("If lacks selected edge"))?;
                }
                Opcode::VariantSwitch => {
                    let variant = match self.value(graph.input(next, 0))? {
                        Value::Variant { variant, .. } => variant,
                        Value::Poison => return Err(InterpretError::PoisonObserved(next)),
                        _ => return Err(malformed("VariantSwitch requires enum value")),
                    };
                    let cases = &graph.variant_switches[graph.payload(next) as usize];
                    let selected = cases
                        .iter()
                        .position(|case| *case == variant)
                        .unwrap_or(cases.len());
                    control = graph
                        .find_projection(next, selected as u32)
                        .ok_or_else(|| malformed("VariantSwitch lacks selected edge"))?;
                }
                Opcode::Jump => {
                    let &(region, slot) = self
                        .interpreter
                        .region_of_jump
                        .get(&next)
                        .ok_or_else(|| malformed("Jump lacks target Region"))?;
                    let phis = self
                        .interpreter
                        .phis_of_region
                        .get(&region)
                        .cloned()
                        .unwrap_or_default();
                    let mut values = Vec::with_capacity(phis.len());
                    for phi in &phis {
                        values.push(if graph.value_type(*phi) == &ValueType::Effect {
                            Value::Scalar(0)
                        } else {
                            self.value(graph.input(*phi, slot + 1))?
                        });
                    }
                    self.phis.extend(phis.into_iter().zip(values));
                    self.cache.clear();
                    control = region;
                }
                Opcode::Return => {
                    let value = self.value(graph.input(next, 1))?;
                    self.check_value(signature.ret, &value)?;
                    if value.contains_poison() {
                        return Err(InterpretError::PoisonObserved(next));
                    }
                    return Ok(SemanticOutcome::Return(value));
                }
                Opcode::Unreachable => return Ok(SemanticOutcome::Unreachable),
                Opcode::TraitCall => {
                    let call = &graph.trait_calls[graph.payload(next) as usize];
                    let mut arguments = Vec::with_capacity(call.signature.args.len());
                    for input in &graph.inputs(next)[1..] {
                        let value = self.value(*input)?;
                        if value.contains_poison() {
                            return Err(InterpretError::PoisonObserved(next));
                        }
                        arguments.push(value);
                    }
                    let HoleAnswer::Resolved { target, .. } = self.answer(call.resolve)? else {
                        return Err(malformed("TraitCall requires resolved target"));
                    };
                    let result = self.calls.call_with_memory(
                        &target.symbol,
                        &arguments,
                        &mut self.memory,
                    )?;
                    self.check_value(call.signature.ret, &result)?;
                    if result.contains_poison() {
                        return Err(InterpretError::PoisonObserved(next));
                    }
                    if let Some(projection) = graph.find_projection(next, 2) {
                        self.fixed.insert(projection, result);
                    }
                    self.cache.clear();
                    control = graph
                        .find_projection(next, 0)
                        .ok_or_else(|| malformed("TraitCall lacks continuation"))?;
                }
                Opcode::Assume => {
                    let predicate = self.value(graph.input(next, 1))?;
                    super::check_assumption(&predicate, next)?;
                    self.cache.clear();
                    control = graph
                        .find_projection(next, 0)
                        .ok_or_else(|| malformed("Assume lacks continuation"))?;
                }
                Opcode::Opaque | Opcode::RequireDefined => {
                    let value = self.value(graph.input(next, 1))?;
                    if graph.op(next) == Opcode::RequireDefined && value.contains_poison() {
                        return Err(InterpretError::PoisonObserved(next));
                    }
                    if graph.op(next) == Opcode::Opaque {
                        if let Some(projection) = graph.find_projection(next, 2) {
                            self.fixed.insert(projection, value);
                        }
                    }
                    self.cache.clear();
                    control = graph
                        .find_projection(next, 0)
                        .ok_or_else(|| malformed("requirement or opaque lacks continuation"))?;
                }
                opcode => {
                    return Err(malformed(format!(
                        "R interpreter does not support dynamic operation {opcode:?}"
                    )));
                }
            }
        }
    }
    fn scalar(&mut self, node: NodeId, observer: NodeId) -> Result<u64> {
        match self.value(node)? {
            Value::Scalar(bits) => Ok(bits),
            Value::Poison => Err(InterpretError::PoisonObserved(observer)),
            _ => Err(malformed("expected scalar value")),
        }
    }
    fn field(&mut self, value: Value, index: u32, ty: TyId) -> Result<Value> {
        match value {
            Value::Aggregate(fields) => fields
                .get(index as usize)
                .cloned()
                .ok_or_else(|| malformed("field out of range")),
            Value::Array { .. } => value
                .array_element(u64::from(index))
                .cloned()
                .ok_or_else(|| malformed("field out of range")),
            Value::WidePointer { data, metadata } => match index {
                0 => Ok(*data),
                1 => Ok(*metadata),
                _ => Err(malformed("wide pointer field out of range")),
            },
            Value::Poison => self.poison(ty),
            _ => Err(malformed("expected product value")),
        }
    }
    fn value(&mut self, node: NodeId) -> Result<Value> {
        if let Some(value) = self
            .phis
            .get(&node)
            .or_else(|| self.fixed.get(&node))
            .or_else(|| self.cache.get(&node))
        {
            return Ok(value.clone());
        }
        let graph = self.interpreter.graph;
        let op = graph.op(node);
        let ty = graph
            .rust_type(node)
            .ok_or_else(|| malformed("R value has no source type"))?;
        if is_anchored_value(op) && graph.control(node) != self.active {
            return Err(malformed("R fixed value used before its control anchor"));
        }
        let value = match op {
            Opcode::Const => {
                if matches!(graph.oracle.semantic().shape(ty), TypeShape::Unit) {
                    Value::Aggregate(vec![])
                } else {
                    Value::Scalar(
                        self.interpreter
                            .normalize(ty, graph.constant_value(node) as u64),
                    )
                }
            }
            Opcode::Undefined => self.poison(ty)?,
            Opcode::Projection => {
                let source = graph.input(node, 0);
                if source == graph.start && graph.payload(node) >= 2 {
                    self.arguments[(graph.payload(node) - 2) as usize].clone()
                } else if graph.op(source) == Opcode::Hole {
                    let HoleAnswer::SizeAlign(layout) = self.answer(graph.payload(source))? else {
                        return Err(malformed("projection requires SizeAlign answer"));
                    };
                    Value::Scalar(if graph.payload(node) == 0 {
                        layout.size
                    } else {
                        layout.align
                    })
                } else {
                    return Err(malformed("R result used before its fixed producer"));
                }
            }
            Opcode::Hole => {
                let HoleAnswer::NeedsDrop(value) = self.answer(graph.payload(node))? else {
                    return Err(malformed("scalar hole requires NeedsDrop answer"));
                };
                Value::Scalar(u64::from(value))
            }
            Opcode::Aggregate | Opcode::MakeVariant | Opcode::WidePtr => {
                let fields = graph
                    .inputs(node)
                    .iter()
                    .map(|input| self.value(*input))
                    .collect::<Result<Vec<_>>>()?;
                match op {
                    Opcode::MakeVariant => Value::Variant {
                        variant: graph.payload(node),
                        fields,
                    },
                    Opcode::WidePtr => {
                        let [data, metadata]: [Value; 2] = fields
                            .try_into()
                            .map_err(|_| malformed("wide pointer components missing"))?;
                        Value::WidePointer {
                            data: Box::new(data),
                            metadata: Box::new(metadata),
                        }
                    }
                    _ => Value::Aggregate(fields),
                }
            }
            Opcode::Repeat => {
                let length = match graph.oracle.semantic().shape(ty) {
                    TypeShape::Array { length, .. } => *length,
                    _ => return Err(malformed("Repeat type is not Array")),
                };
                Value::array(
                    if length == 0 {
                        Value::Poison
                    } else {
                        self.value(graph.input(node, 0))?
                    },
                    length,
                )
            }
            Opcode::Field => {
                let value = self.value(graph.input(node, 0))?;
                self.field(value, graph.payload(node), ty)?
            }
            Opcode::InsertField => {
                let mut value = self.value(graph.input(node, 0))?;
                if value == Value::Poison {
                    value = self.poison(ty)?;
                }
                let replacement = self.value(graph.input(node, 1))?;
                let index = graph.payload(node);
                match &mut value {
                    Value::Aggregate(fields) => {
                        *fields
                            .get_mut(index as usize)
                            .ok_or_else(|| malformed("insert field out of range"))? = replacement
                    }
                    Value::Array {
                        element,
                        length,
                        overrides,
                    } if u64::from(index) < *length => {
                        let overrides = Arc::make_mut(overrides);
                        if &replacement == element.as_ref() {
                            overrides.remove(&u64::from(index));
                        } else {
                            overrides.insert(u64::from(index), replacement);
                        }
                    }
                    Value::WidePointer { data, metadata } => match index {
                        0 => **data = replacement,
                        1 => **metadata = replacement,
                        _ => return Err(malformed("wide pointer field out of range")),
                    },
                    _ => return Err(malformed("InsertField requires product value")),
                }
                value
            }
            Opcode::VariantField => {
                let (expected, field) = graph.variant_fields[graph.payload(node) as usize];
                let value = self.value(graph.input(node, 0))?;
                super::check_variant_witness(graph, node, &value)?;
                match value {
                    Value::Variant { variant, fields } if variant == expected => fields
                        .get(field as usize)
                        .cloned()
                        .ok_or_else(|| malformed("variant field out of range"))?,
                    Value::Variant { .. } | Value::Poison => self.poison(ty)?,
                    _ => return Err(malformed("VariantField requires enum")),
                }
            }
            Opcode::VariantOf | Opcode::Discriminant => match self.value(graph.input(node, 0))? {
                Value::Variant { variant, .. } => Value::Scalar(if op == Opcode::VariantOf {
                    u64::from(variant)
                } else {
                    graph
                        .oracle
                        .semantic()
                        .discriminant(
                            graph.rust_type(graph.input(node, 0)).expect("typed enum"),
                            variant,
                        )
                        .ok_or_else(|| malformed("discriminant absent"))? as u64
                }),
                Value::Poison => Value::Poison,
                _ => return Err(malformed("variant query requires enum")),
            },
            op if op.is_binary_arithmetic()
                || matches!(
                    op,
                    Opcode::Negate
                        | Opcode::BitNot
                        | Opcode::IntCast
                        | Opcode::Compare
                        | Opcode::MulOverflow
                ) =>
            {
                let mut operands = Vec::new();
                for input in graph.inputs(node) {
                    operands.push(match self.value(*input)? {
                        Value::Scalar(bits) => Scalar::new(bits),
                        Value::Poison => Scalar {
                            bits: 0,
                            poison: true,
                        },
                        _ => return Err(malformed("scalar operation has aggregate operand")),
                    });
                }
                let result = self
                    .interpreter
                    .apply(node, op, ty, graph.inputs(node), &operands)?;
                if result.poison {
                    Value::Poison
                } else {
                    Value::Scalar(result.bits)
                }
            }
            opcode => {
                return Err(malformed(format!(
                    "R interpreter does not support value operation {opcode:?}"
                )));
            }
        };
        if is_anchored_value(op) {
            self.fixed.insert(node, value.clone());
        } else {
            self.cache.insert(node, value.clone());
        }
        Ok(value)
    }
}
