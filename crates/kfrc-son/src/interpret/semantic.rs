//! Layout-independent values and execution for the specialized semantic graph.
//!
//! Aggregate values own their fields. Reading or copying one cannot alias a
//! subsequent InsertField, and poison remains confined to the affected leaves.

use super::semantic_memory::physical_fields;
use super::{InterpretError, Interpreter, Scalar, SemanticMemory, Value};
use crate::analysis::cfg::is_anchored_value;
use crate::graph::{GraphLevel, NodeId, Opcode, ValueType};
use kfrc_body::fx_hash::HashMap;
use kfrc_body::{TyId, TypeOracle, TypeShape};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SemanticOutcome {
    Return(Value),
    Unreachable,
}

/// Calls use one value per semantic argument, independent of physical ABI lanes.
pub trait SemanticCallHandler {
    fn call(&mut self, symbol: &str, arguments: &[Value]) -> Result<Value, InterpretError>;

    fn call_with_memory(
        &mut self,
        symbol: &str,
        arguments: &[Value],
        _memory: &mut SemanticMemory,
    ) -> Result<Value, InterpretError> {
        self.call(symbol, arguments)
    }

    fn call_indirect_with_memory(
        &mut self,
        address: u64,
        arguments: &[Value],
        _memory: &mut SemanticMemory,
    ) -> Result<Value, InterpretError> {
        self.call_indirect(address, arguments)
    }

    fn call_indirect(
        &mut self,
        address: u64,
        _arguments: &[Value],
    ) -> Result<Value, InterpretError> {
        Err(InterpretError::UnknownCallee(format!(
            "indirect@{address:#x}"
        )))
    }
}

impl<F> SemanticCallHandler for F
where
    F: FnMut(&str, &[Value]) -> Result<Value, InterpretError>,
{
    fn call(&mut self, symbol: &str, arguments: &[Value]) -> Result<Value, InterpretError> {
        self(symbol, arguments)
    }
}

struct ValueFrame<'a, 'm> {
    arguments: &'a [Value],
    phis: HashMap<NodeId, Value>,
    fixed: HashMap<NodeId, Value>,
    cache: HashMap<NodeId, Value>,
    active_control: NodeId,
    memory: &'m mut SemanticMemory,
    slots: HashMap<u32, u64>,
    symbols: HashMap<String, u64>,
}

impl Interpreter<'_> {
    /// Execute an S graph without interpreting aggregates as addresses.
    ///
    /// `arguments` includes every logical parameter, including zero-sized ones.
    /// Physical ABI marshaling belongs to layout lowering and the L-level `run`.
    pub fn run_values(
        &mut self,
        arguments: &[Value],
        calls: &mut dyn SemanticCallHandler,
    ) -> Result<SemanticOutcome, InterpretError> {
        self.run_values_in_memory(arguments, calls, &mut SemanticMemory::new())
    }

    pub fn run_values_in_memory(
        &mut self,
        arguments: &[Value],
        calls: &mut dyn SemanticCallHandler,
        memory: &mut SemanticMemory,
    ) -> Result<SemanticOutcome, InterpretError> {
        let graph = self.graph;
        if graph.level != GraphLevel::Specialized {
            return Err(malformed("run_values requires a Specialized graph"));
        }
        crate::verify::check(graph).map_err(|error| malformed(error.to_string()))?;
        if arguments.len() != graph.abi.args.len() {
            return Err(malformed("semantic argument count differs from signature"));
        }
        let ValueType::Tuple(start_types) = graph.value_type(graph.start) else {
            return Err(malformed("Start does not have a result tuple"));
        };
        for (index, argument) in arguments.iter().enumerate() {
            let Some(ValueType::Rust(ty)) = start_types.get(index + 2) else {
                return Err(malformed("parameter does not have a semantic Rust type"));
            };
            self.check_value(*ty, argument)?;
            if argument.contains_poison() {
                return Err(InterpretError::PoisonObserved(graph.start));
            }
        }
        let mut frame = ValueFrame {
            arguments,
            phis: HashMap::default(),
            fixed: HashMap::default(),
            cache: HashMap::default(),
            active_control: NodeId::NONE,
            memory,
            slots: HashMap::default(),
            symbols: HashMap::default(),
        };
        frame.memory.register_data(&graph.data_objects)?;
        for (symbol, _) in &graph.symbols {
            let address = frame.memory.symbol_address(symbol)?;
            frame.symbols.insert(symbol.clone(), address);
        }
        for node in graph
            .ids()
            .filter(|node| graph.op(*node) == Opcode::LocalSlot)
        {
            let ty = self.pointee(node)?;
            let layout = graph
                .oracle
                .size_align(ty)
                .ok_or_else(|| malformed("local type has no layout"))?;
            let address = frame.memory.allocate(layout.size, layout.align, true)?;
            frame.slots.insert(graph.payload(node), address);
        }
        let mut control = graph
            .find_projection(graph.start, 0)
            .ok_or_else(|| malformed("Start lacks control projection"))?;
        let outcome = (|| {
            loop {
                if self.fuel == 0 {
                    return Err(InterpretError::OutOfFuel);
                }
                self.fuel -= 1;
                let users = self
                    .control_users
                    .get(&control)
                    .cloned()
                    .unwrap_or_default();
                frame.active_control = control;
                for user in &users {
                    if is_anchored_value(graph.op(*user)) {
                        frame.fixed.remove(user);
                    }
                }
                for user in &users {
                    if is_anchored_value(graph.op(*user)) {
                        self.evaluate_value(*user, &mut frame, false)?;
                    }
                }
                let next = users
                    .into_iter()
                    .find(|node| {
                        matches!(
                            graph.op(*node),
                            Opcode::If
                                | Opcode::VariantSwitch
                                | Opcode::Jump
                                | Opcode::Return
                                | Opcode::Unreachable
                                | Opcode::Call
                                | Opcode::CallIndirect
                                | Opcode::Opaque
                                | Opcode::Load
                                | Opcode::Store
                                | Opcode::ThreadLocalAddress
                                | Opcode::LoadValue
                                | Opcode::StoreValue
                                | Opcode::SetVariant
                                | Opcode::SlotLive
                                | Opcode::SlotDead
                                | Opcode::CopyNonOverlapping
                                | Opcode::RequireDefined
                                | Opcode::Assume
                                | Opcode::TransmuteValue
                        )
                    })
                    .ok_or_else(|| malformed(format!("control {control} has no successor")))?;
                match graph.op(next) {
                    Opcode::VariantSwitch => {
                        let value = self.evaluate_value(graph.input(next, 0), &mut frame, false)?;
                        let variant = match value {
                            Value::Variant { variant, .. } => variant,
                            Value::Poison => return Err(InterpretError::PoisonObserved(next)),
                            _ => return Err(malformed("VariantSwitch input is not an enum value")),
                        };
                        let cases = &graph.variant_switches[graph.payload(next) as usize];
                        let branch = cases
                            .iter()
                            .position(|case| *case == variant)
                            .unwrap_or(cases.len()) as u32;
                        control = graph
                            .find_projection(next, branch)
                            .ok_or_else(|| malformed("VariantSwitch lacks selected projection"))?;
                    }
                    Opcode::If => {
                        let condition =
                            self.evaluate_value(graph.input(next, 0), &mut frame, false)?;
                        let condition = observed_scalar(condition, next)?;
                        let branch = u32::from(condition & 1 == 0);
                        control = graph
                            .find_projection(next, branch)
                            .ok_or_else(|| malformed("If lacks selected projection"))?;
                    }
                    Opcode::Jump => {
                        let (region, slot) = *self
                            .region_of_jump
                            .get(&next)
                            .ok_or_else(|| malformed("Jump has no Region"))?;
                        let phis = self
                            .phis_of_region
                            .get(&region)
                            .cloned()
                            .unwrap_or_default();
                        let mut incoming = Vec::with_capacity(phis.len());
                        for phi in &phis {
                            let value = if graph.value_type(*phi) == &ValueType::Effect {
                                Value::Scalar(0)
                            } else {
                                self.evaluate_value(graph.input(*phi, slot + 1), &mut frame, false)?
                            };
                            incoming.push(value);
                        }
                        // All incoming values observe the previous iteration's Phi values.
                        for (phi, value) in phis.into_iter().zip(incoming) {
                            frame.phis.insert(phi, value);
                        }
                        frame.cache.clear();
                        control = region;
                    }
                    Opcode::Return => {
                        let source = graph.input(next, 1);
                        let value = self.evaluate_value(source, &mut frame, false)?;
                        let ty = graph
                            .rust_type(source)
                            .ok_or_else(|| malformed("Return lacks semantic source type"))?;
                        self.check_value(ty, &value)?;
                        if value.contains_poison() {
                            return Err(InterpretError::PoisonObserved(next));
                        }
                        return Ok(SemanticOutcome::Return(value));
                    }
                    Opcode::Unreachable => return Ok(SemanticOutcome::Unreachable),
                    Opcode::LoadValue
                    | Opcode::StoreValue
                    | Opcode::SetVariant
                    | Opcode::SlotLive
                    | Opcode::SlotDead
                    | Opcode::CopyNonOverlapping => {
                        self.execute_memory(next, &mut frame)?;
                        frame.cache.clear();
                        control = self.semantic_continuation(next)?;
                    }
                    Opcode::TransmuteValue => {
                        let source = graph.input(next, 1);
                        let source_ty = graph
                            .rust_type(source)
                            .ok_or_else(|| malformed("transmute source type missing"))?;
                        let ValueType::Tuple(results) = graph.value_type(next) else {
                            return Err(malformed("transmute result tuple missing"));
                        };
                        let Some(ValueType::Rust(target_ty)) = results.get(2) else {
                            return Err(malformed("transmute result type missing"));
                        };
                        let from = graph
                            .oracle
                            .size_align(source_ty)
                            .ok_or_else(|| malformed("transmute source layout missing"))?;
                        let to = graph
                            .oracle
                            .size_align(*target_ty)
                            .ok_or_else(|| malformed("transmute target layout missing"))?;
                        if from.size != to.size {
                            return Err(malformed("transmute source and target sizes differ"));
                        }
                        let value = self.evaluate_value(source, &mut frame, false)?;
                        if value.contains_poison() {
                            return Err(InterpretError::PoisonObserved(next));
                        }
                        let temporary =
                            frame
                                .memory
                                .allocate(from.size, from.align.max(to.align), true)?;
                        frame
                            .memory
                            .store(&graph.oracle, temporary, source_ty, &value)?;
                        let value = frame.memory.load(&graph.oracle, temporary, *target_ty)?;
                        if value.contains_poison() {
                            return Err(InterpretError::PoisonObserved(next));
                        }
                        if let Some(result) = graph.find_projection(next, 2) {
                            frame.fixed.insert(result, value);
                        }
                        frame.cache.clear();
                        control = self.semantic_continuation(next)?;
                    }
                    Opcode::Assume => {
                        let predicate =
                            self.evaluate_value(graph.input(next, 1), &mut frame, false)?;
                        super::check_assumption(&predicate, next)?;
                        frame.cache.clear();
                        control = self.semantic_continuation(next)?;
                    }
                    Opcode::Opaque | Opcode::RequireDefined => {
                        let value = self.evaluate_value(graph.input(next, 1), &mut frame, false)?;
                        if graph.op(next) == Opcode::RequireDefined && value.contains_poison() {
                            return Err(InterpretError::PoisonObserved(next));
                        }
                        if graph.op(next) == Opcode::Opaque {
                            if let Some(result) = graph.find_projection(next, 2) {
                                frame.fixed.insert(result, value);
                            }
                        }
                        frame.cache.clear();
                        control = self.semantic_continuation(next)?;
                    }
                    Opcode::Call | Opcode::CallIndirect => {
                        let indirect = graph.op(next) == Opcode::CallIndirect;
                        let mut values = Vec::new();
                        for argument in &graph.inputs(next)[if indirect { 2 } else { 1 }..] {
                            let value = self.evaluate_value(*argument, &mut frame, false)?;
                            if value.contains_poison() {
                                return Err(InterpretError::PoisonObserved(next));
                            }
                            values.push(value);
                        }
                        let value = if indirect {
                            let address =
                                self.evaluate_value(graph.input(next, 1), &mut frame, false)?;
                            calls.call_indirect_with_memory(
                                observed_scalar(address, next)?,
                                &values,
                                frame.memory,
                            )?
                        } else {
                            let target = &graph.callees[graph.payload(next) as usize];
                            calls.call_with_memory(&target.symbol, &values, frame.memory)?
                        };
                        let ValueType::Tuple(results) = graph.value_type(next) else {
                            return Err(malformed("Call result lacks tuple"));
                        };
                        let Some(ValueType::Rust(ty)) = results.get(2) else {
                            return Err(malformed("Call result lacks semantic type"));
                        };
                        self.check_value(*ty, &value)?;
                        if value.contains_poison() {
                            return Err(InterpretError::PoisonObserved(next));
                        }
                        if let Some(result) = graph.find_projection(next, 2) {
                            frame.fixed.insert(result, value);
                        }
                        frame.cache.clear();
                        control = self.semantic_continuation(next)?;
                    }
                    other => {
                        return Err(malformed(format!(
                            "semantic interpreter does not yet execute {other:?}"
                        )));
                    }
                }
            }
        })();
        // Stack storage belongs to this invocation even when a raw pointer to
        // it escapes, or interpretation ends through UB/error/divergence.
        for address in frame.slots.values() {
            frame.memory.lifetime(*address, false)?;
        }
        outcome
    }

    fn pointee(&self, node: NodeId) -> Result<TyId, InterpretError> {
        let ty = self
            .graph
            .rust_type(node)
            .ok_or_else(|| malformed("pointer lacks type"))?;
        self.graph
            .oracle
            .semantic()
            .thin_pointee(ty)
            .ok_or_else(|| malformed("expected thin storage address"))
    }

    fn execute_memory(
        &self,
        node: NodeId,
        frame: &mut ValueFrame<'_, '_>,
    ) -> Result<(), InterpretError> {
        let graph = self.graph;
        let pointer = graph.input(node, 1);
        let address = observed_scalar(self.evaluate_value(pointer, frame, false)?, node)?;
        let ty = self.pointee(pointer)?;
        match graph.op(node) {
            Opcode::LoadValue => {
                let value = frame.memory.load(&graph.oracle, address, ty)?;
                if let Some(result) = graph.find_projection(node, 2) {
                    frame.fixed.insert(result, value);
                }
            }
            Opcode::StoreValue => {
                let value = self.evaluate_value(graph.input(node, 2), frame, false)?;
                frame.memory.store(&graph.oracle, address, ty, &value)?;
            }
            Opcode::SetVariant => {
                let variant = observed_scalar(
                    self.evaluate_value(graph.input(node, 2), frame, false)?,
                    node,
                )?;
                let variant =
                    u32::try_from(variant).map_err(|_| malformed("variant number too large"))?;
                frame
                    .memory
                    .set_variant(&graph.oracle, address, ty, variant)?;
            }
            Opcode::SlotLive | Opcode::SlotDead => frame
                .memory
                .lifetime(address, graph.op(node) == Opcode::SlotLive)?,
            Opcode::CopyNonOverlapping => {
                let destination = observed_scalar(
                    self.evaluate_value(graph.input(node, 2), frame, false)?,
                    node,
                )?;
                let count = observed_scalar(
                    self.evaluate_value(graph.input(node, 3), frame, false)?,
                    node,
                )?;
                let layout = graph
                    .oracle
                    .size_align(ty)
                    .ok_or_else(|| malformed("copy element lacks layout"))?;
                let size = layout
                    .size
                    .checked_mul(count)
                    .ok_or_else(|| malformed("copy size overflow"))?;
                frame
                    .memory
                    .copy_nonoverlapping(address, destination, size, layout.align)?;
            }
            _ => return Err(malformed("not a semantic memory operation")),
        }
        Ok(())
    }

    fn semantic_continuation(&self, node: NodeId) -> Result<NodeId, InterpretError> {
        self.graph
            .find_projection(node, 0)
            .ok_or_else(|| malformed(format!("{node} lacks continuation")))
    }

    fn evaluate_value(
        &self,
        node: NodeId,
        frame: &mut ValueFrame<'_, '_>,
        executing_fixed: bool,
    ) -> Result<Value, InterpretError> {
        let graph = self.graph;
        if !executing_fixed {
            if let Some(value) = frame.phis.get(&node).or_else(|| frame.fixed.get(&node)) {
                return Ok(value.clone());
            }
            if let Some(value) = frame.cache.get(&node) {
                return Ok(value.clone());
            }
        }
        if !executing_fixed && is_anchored_value(graph.op(node)) {
            if graph.control(node) != frame.active_control {
                return Err(malformed(format!("{node} read before its control anchor")));
            }
            let value = self.evaluate_value(node, frame, true)?;
            frame.fixed.insert(node, value.clone());
            return Ok(value);
        }
        let ty = graph
            .rust_type(node)
            .ok_or_else(|| malformed("value lacks Rust type"))?;
        let op = graph.op(node);
        let value = match op {
            Opcode::Const => {
                if matches!(graph.oracle.semantic().shape(ty), TypeShape::Unit) {
                    Value::Aggregate(Vec::new())
                } else {
                    Value::Scalar(self.normalize(ty, graph.constant_value(node) as u64))
                }
            }
            Opcode::ConstRepr => {
                let layout = graph
                    .oracle
                    .size_align(ty)
                    .ok_or_else(|| malformed("constant representation layout missing"))?;
                if layout.size > 16 {
                    return Err(malformed("constant representation exceeds u128"));
                }
                let bits = graph.constant_value(node);
                if layout.size < 16 && bits >> (layout.size * 8) != 0 {
                    return Err(malformed("constant representation has excess high bits"));
                }
                let address = frame.memory.allocate(layout.size, layout.align, true)?;
                frame
                    .memory
                    .initialize_bytes(address, &bits.to_le_bytes()[..layout.size as usize]);
                frame.memory.load(&graph.oracle, address, ty)?
            }
            Opcode::Undefined => self.poison_value(ty)?,
            Opcode::LocalSlot => Value::Scalar(
                *frame
                    .slots
                    .get(&graph.payload(node))
                    .ok_or_else(|| malformed("missing local allocation"))?,
            ),
            Opcode::SymbolAddress => {
                let (symbol, addend) = &graph.symbols[graph.payload(node) as usize];
                Value::Scalar(
                    frame
                        .symbols
                        .get(symbol)
                        .ok_or_else(|| malformed("symbol missing"))?
                        .wrapping_add_signed(*addend),
                )
            }
            Opcode::PointerCast => self.evaluate_value(graph.input(node, 0), frame, false)?,
            Opcode::FieldAddr | Opcode::VariantFieldAddr | Opcode::IndexAddr => {
                let base_node = graph.input(node, 0);
                let base = self.evaluate_value(base_node, frame, false)?;
                if base == Value::Poison {
                    Value::Poison
                } else {
                    let address = observed_scalar(base, node)?;
                    let offset = if op == Opcode::IndexAddr {
                        let index = self.evaluate_value(graph.input(node, 1), frame, false)?;
                        if index == Value::Poison {
                            return Ok(Value::Poison);
                        }
                        let index = observed_scalar(index, node)?;
                        let stride = graph
                            .oracle
                            .size_align(self.pointee(node)?)
                            .ok_or_else(|| malformed("index element layout missing"))?
                            .size;
                        index.wrapping_mul(stride)
                    } else {
                        let (variant, field) = if op == Opcode::VariantFieldAddr {
                            let (variant, field) =
                                graph.variant_fields[graph.payload(node) as usize];
                            (Some(variant), field)
                        } else {
                            (None, graph.payload(node))
                        };
                        let source = self.pointee(base_node)?;
                        if let TypeShape::Array { element, .. } =
                            graph.oracle.semantic().shape(source)
                        {
                            let stride = graph
                                .oracle
                                .size_align(*element)
                                .ok_or_else(|| malformed("array element layout missing"))?
                                .size;
                            u64::from(field) * stride
                        } else {
                            physical_fields(&graph.oracle, source, variant)?
                                .get(field as usize)
                                .ok_or_else(|| malformed("address field missing"))?
                                .offset
                        }
                    };
                    Value::Scalar(address.wrapping_add(offset))
                }
            }
            Opcode::Projection => {
                if graph.input(node, 0) != graph.start || graph.payload(node) < 2 {
                    return Err(malformed(format!(
                        "{node} read before its fixed source ran"
                    )));
                }
                frame
                    .arguments
                    .get(graph.payload(node) as usize - 2)
                    .cloned()
                    .ok_or_else(|| malformed("missing semantic argument"))?
            }
            Opcode::Phi => return Err(malformed(format!("{node} read before its Region ran"))),
            Opcode::Repeat => {
                let length = match graph.oracle.semantic().shape(ty) {
                    TypeShape::Array { length, .. } => *length,
                    _ => return Err(malformed("Repeat result is not an array")),
                };
                let element = if length == 0 {
                    Value::Poison
                } else {
                    self.evaluate_value(graph.input(node, 0), frame, false)?
                };
                Value::array(element, length)
            }
            Opcode::Aggregate | Opcode::MakeVariant | Opcode::WidePtr => {
                let mut fields = Vec::with_capacity(graph.inputs(node).len());
                for input in graph.inputs(node) {
                    fields.push(self.evaluate_value(*input, frame, false)?);
                }
                match op {
                    Opcode::MakeVariant => Value::Variant {
                        variant: graph.payload(node),
                        fields,
                    },
                    Opcode::WidePtr => {
                        let mut fields = fields.into_iter();
                        Value::WidePointer {
                            data: Box::new(
                                fields
                                    .next()
                                    .ok_or_else(|| malformed("missing wide pointer data"))?,
                            ),
                            metadata: Box::new(
                                fields
                                    .next()
                                    .ok_or_else(|| malformed("missing wide pointer metadata"))?,
                            ),
                        }
                    }
                    _ => Value::Aggregate(fields),
                }
            }
            Opcode::Field => {
                let value = self.evaluate_value(graph.input(node, 0), frame, false)?;
                self.value_field(value, graph.payload(node), ty)?
            }
            Opcode::InsertField => {
                let mut value = self.evaluate_value(graph.input(node, 0), frame, false)?;
                if value == Value::Poison {
                    value = self.poison_value(ty)?;
                }
                let field = self.evaluate_value(graph.input(node, 1), frame, false)?;
                let index = graph.payload(node) as usize;
                match &mut value {
                    Value::Aggregate(fields) => {
                        *fields
                            .get_mut(index)
                            .ok_or_else(|| malformed("InsertField out of range"))? = field;
                    }
                    Value::Array {
                        element,
                        length,
                        overrides,
                    } => {
                        if index as u64 >= *length {
                            return Err(malformed("InsertField out of range"));
                        }
                        let overrides = Arc::make_mut(overrides);
                        if &field == element.as_ref() {
                            overrides.remove(&(index as u64));
                        } else {
                            overrides.insert(index as u64, field);
                        }
                    }
                    Value::WidePointer { data, metadata } => match index {
                        0 => **data = field,
                        1 => **metadata = field,
                        _ => return Err(malformed("wide pointer field out of range")),
                    },
                    _ => return Err(malformed("InsertField requires aggregate")),
                }
                value
            }
            Opcode::VariantOf | Opcode::Discriminant => {
                let source = graph.input(node, 0);
                match self.evaluate_value(source, frame, false)? {
                    Value::Variant { variant, .. } => {
                        let bits = if op == Opcode::VariantOf {
                            u128::from(variant)
                        } else {
                            let source_ty = graph
                                .rust_type(source)
                                .ok_or_else(|| malformed("enum type missing"))?;
                            graph
                                .oracle
                                .semantic()
                                .discriminant(source_ty, variant)
                                .ok_or_else(|| malformed("enum discriminant missing"))?
                        };
                        Value::Scalar(self.normalize(ty, bits as u64))
                    }
                    Value::Poison => Value::Poison,
                    _ => return Err(malformed("variant query requires enum value")),
                }
            }
            Opcode::VariantField => {
                let (variant, field) = graph.variant_fields[graph.payload(node) as usize];
                let value = self.evaluate_value(graph.input(node, 0), frame, false)?;
                super::check_variant_witness(graph, node, &value)?;
                match value {
                    Value::Variant {
                        variant: active,
                        fields,
                    } if active == variant => fields
                        .get(field as usize)
                        .cloned()
                        .ok_or_else(|| malformed("variant field out of range"))?,
                    Value::Variant { .. } | Value::Poison => self.poison_value(ty)?,
                    _ => return Err(malformed("VariantField requires enum value")),
                }
            }
            op if is_trapping(op) && !executing_fixed => {
                return Err(malformed(format!("{node} read before control reached it")));
            }
            _ => {
                let inputs = graph.inputs(node);
                let mut scalars = Vec::with_capacity(inputs.len());
                for input in inputs {
                    scalars.push(match self.evaluate_value(*input, frame, false)? {
                        Value::Scalar(bits) => Scalar::new(bits),
                        Value::Poison => Scalar {
                            bits: 0,
                            poison: true,
                        },
                        _ => return Err(malformed("scalar operation received aggregate value")),
                    });
                }
                let result = self.apply(node, op, ty, inputs, &scalars)?;
                if result.poison {
                    Value::Poison
                } else {
                    Value::Scalar(result.bits)
                }
            }
        };
        if !executing_fixed {
            frame.cache.insert(node, value.clone());
        }
        Ok(value)
    }

    fn value_field(&self, value: Value, index: u32, ty: TyId) -> Result<Value, InterpretError> {
        match value {
            Value::Aggregate(fields) => fields
                .get(index as usize)
                .cloned()
                .ok_or_else(|| malformed("field index out of range")),
            Value::Array { .. } => value
                .array_element(u64::from(index))
                .cloned()
                .ok_or_else(|| malformed("field index out of range")),
            Value::WidePointer { data, metadata } => match index {
                0 => Ok(*data),
                1 => Ok(*metadata),
                _ => Err(malformed("wide pointer field index out of range")),
            },
            Value::Poison => self.poison_value(ty),
            _ => Err(malformed("Field requires aggregate or wide pointer")),
        }
    }

    fn poison_value(&self, ty: TyId) -> Result<Value, InterpretError> {
        super::value::poison(&self.graph.oracle, ty)
    }

    fn check_value(&self, ty: TyId, value: &Value) -> Result<(), InterpretError> {
        super::value::check(&self.graph.oracle, ty, value)
    }
}

fn observed_scalar(value: Value, node: NodeId) -> Result<u64, InterpretError> {
    match value {
        Value::Scalar(bits) => Ok(bits),
        Value::Poison => Err(InterpretError::PoisonObserved(node)),
        _ => Err(malformed("expected scalar value")),
    }
}

fn is_trapping(op: Opcode) -> bool {
    matches!(
        op,
        Opcode::DivSigned | Opcode::DivUnsigned | Opcode::RemSigned | Opcode::RemUnsigned
    )
}

fn malformed(detail: impl Into<String>) -> InterpretError {
    InterpretError::Malformed(detail.into())
}
