//! Reference interpreters for layout and specialized semantic graphs.
//!
//! Fixed nodes run in control order; pure nodes are evaluated on demand;
//! all `Phi`s of a `Region` take their values at once when control enters
//! it. It locates miscompiles (run a graph before and after a pass on the
//! same input); correctness is still judged by real execution.

use kfrc_body::fx_hash::{HashMap, HashSet};

mod rust;
mod semantic;
mod semantic_memory;
mod value;
pub use semantic_memory::SemanticMemory;
mod tls;
pub use semantic::{SemanticCallHandler, SemanticOutcome};
pub use tls::InterpreterThread;
pub use value::Value;

use kfrc_body::{IntegerWidth, TyId, TypeOracle, TypeShape};

use crate::analysis::cfg::is_anchored_value;
use crate::graph::{ComparePredicate, FunctionGraph, NodeId, Opcode, ValueType};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The returned value's bits (zero-extended), or 0 for ignored returns.
    Return(u64),
    ReturnPair(u64, u64),
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

    fn call_pair(
        &mut self,
        symbol: &str,
        _arguments: &[u64],
    ) -> Result<(u64, u64), InterpretError> {
        Err(InterpretError::Malformed(format!(
            "callee {symbol} has no pair-return interpreter"
        )))
    }
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
    thread: Option<InterpreterThread>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Scalar {
    bits: u64,
    poison: bool,
}

impl Scalar {
    fn new(bits: u64) -> Self {
        Self {
            bits,
            poison: false,
        }
    }
}

struct Frame {
    phi_values: HashMap<NodeId, Scalar>,
    fixed_values: HashMap<NodeId, Scalar>,
    cache: HashMap<NodeId, Scalar>,
    active_control: NodeId,
    arguments: Vec<u64>,
    slots: Vec<u64>,
    memory: HashMap<u64, u8>,
    symbols: HashMap<String, u64>,
    dead_slots: HashSet<usize>,
}

impl<'a> Interpreter<'a> {
    pub fn new(graph: &'a FunctionGraph) -> Self {
        let mut control_users: HashMap<NodeId, Vec<NodeId>> = HashMap::default();
        let mut region_of_jump = HashMap::default();
        let mut phis_of_region: HashMap<NodeId, Vec<NodeId>> = HashMap::default();
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
                Opcode::Phi => phis_of_region
                    .entry(graph.input(node, 0))
                    .or_default()
                    .push(node),
                _ => {}
            }
        }
        Self {
            graph,
            control_users,
            region_of_jump,
            phis_of_region,
            fuel: 10_000_000,
            thread: Some(InterpreterThread::new()),
        }
    }

    pub fn run(
        &mut self,
        arguments: &[u64],
        calls: &mut dyn CallHandler,
    ) -> Result<Outcome, InterpretError> {
        let mut thread = self.thread.take().expect("interpreter thread context");
        let outcome = self.run_in_thread(arguments, calls, &mut thread);
        self.thread = Some(thread);
        outcome
    }

    /// Execute in an explicitly shared logical thread, retaining TLS across
    /// calls to different interpreted functions while isolating other threads.
    pub fn run_in_thread(
        &mut self,
        arguments: &[u64],
        calls: &mut dyn CallHandler,
        thread: &mut InterpreterThread,
    ) -> Result<Outcome, InterpretError> {
        if self.graph.level != crate::graph::GraphLevel::Layout {
            return Err(InterpretError::Malformed(
                "run requires a layout graph; use run_values for Specialized graphs".to_owned(),
            ));
        }
        let graph = self.graph;
        let mut frame = Frame {
            phi_values: HashMap::default(),
            fixed_values: HashMap::default(),
            cache: HashMap::default(),
            active_control: NodeId::NONE,
            arguments: arguments.to_vec(),
            slots: Vec::new(),
            memory: HashMap::default(),
            symbols: HashMap::default(),
            dead_slots: HashSet::default(),
        };
        let mut next_address = 0x1000u64;
        for slot in &graph.stack_slots {
            next_address = next_address.next_multiple_of(slot.align.max(1));
            frame.slots.push(next_address);
            next_address += slot.size.max(1) + 16;
        }
        let tls_symbols: HashSet<_> = graph
            .ids()
            .filter(|node| graph.op(*node) == Opcode::ThreadLocalAddress)
            .map(|node| graph.symbols[graph.payload(node) as usize].0.clone())
            .chain(
                graph
                    .data_objects
                    .iter()
                    .filter(|object| object.thread_local)
                    .map(|object| object.symbol.clone()),
            )
            .collect();
        let mut new_tls = HashSet::default();
        for object in &graph.data_objects {
            if object.thread_local {
                let (address, fresh) = thread.ensure(object)?;
                frame.symbols.insert(object.symbol.clone(), address);
                if fresh {
                    new_tls.insert(object.symbol.clone());
                }
            } else {
                next_address = next_address.next_multiple_of(object.align.max(1));
                frame.symbols.insert(object.symbol.clone(), next_address);
                next_address += object.bytes.len() as u64 + 16;
            }
        }
        thread.restore(&mut frame.memory);
        for (symbol, _) in &graph.symbols {
            if !frame.symbols.contains_key(symbol) {
                if tls_symbols.contains(symbol) {
                    if let Some(address) = thread.address(symbol) {
                        frame.symbols.insert(symbol.clone(), address);
                    }
                } else {
                    frame.symbols.insert(symbol.clone(), next_address);
                    next_address += 16;
                }
            }
        }
        for object in &graph.data_objects {
            for relocation in &object.relocations {
                if tls_symbols.contains(&relocation.target) {
                    return Err(InterpretError::Malformed(
                        "ordinary data relocation cannot target TLS".to_owned(),
                    ));
                }
                if !frame.symbols.contains_key(&relocation.target) {
                    frame
                        .symbols
                        .insert(relocation.target.clone(), next_address);
                    next_address += 16;
                }
            }
        }
        for object in &graph.data_objects {
            if object.thread_local && !new_tls.contains(&object.symbol) {
                continue;
            }
            let base = frame.symbols[&object.symbol];
            for (offset, byte) in object.bytes.iter().enumerate() {
                frame.memory.insert(base + offset as u64, *byte);
            }
            for relocation in &object.relocations {
                let target = frame.symbols.get(&relocation.target).ok_or_else(|| {
                    InterpretError::Malformed(format!("unknown data symbol {}", relocation.target))
                })?;
                let value = target.wrapping_add_signed(relocation.addend);
                for (offset, byte) in value.to_le_bytes().into_iter().enumerate() {
                    frame
                        .memory
                        .insert(base + relocation.offset + offset as u64, byte);
                }
            }
        }
        let outcome = self.run_frame(arguments, calls, thread, &mut frame, next_address);
        thread.save(&frame.memory);
        outcome
    }

    fn run_frame(
        &mut self,
        arguments: &[u64],
        calls: &mut dyn CallHandler,
        thread: &InterpreterThread,
        frame: &mut Frame,
        mut next_address: u64,
    ) -> Result<Outcome, InterpretError> {
        let graph = self.graph;
        let mut argument_index = 0;
        for (index, mode) in graph.abi.args.iter().enumerate() {
            let Some(parameter) = graph.find_projection(graph.start, 2 + index as u32) else {
                argument_index += match mode {
                    kfrc_body::PassMode::Ignore => 0,
                    kfrc_body::PassMode::Pair | kfrc_body::PassMode::Vector { bytes: 16 } => 2,
                    kfrc_body::PassMode::Cast(abi) => cast_argument_count(*abi),
                    _ => 1,
                };
                continue;
            };
            if let kfrc_body::PassMode::Cast(abi) = mode {
                let ty = graph.rust_type(parameter).ok_or_else(|| {
                    InterpretError::Malformed("cast parameter lacks type".to_owned())
                })?;
                let layout = graph.oracle.size_align(ty).ok_or_else(|| {
                    InterpretError::Malformed("cast parameter lacks layout".to_owned())
                })?;
                let count = cast_argument_count(*abi);
                let input = arguments
                    .get(argument_index..argument_index + count)
                    .ok_or_else(|| {
                        InterpretError::Malformed("too few cast parameter lanes".to_owned())
                    })?;
                argument_index += count;
                let address = write_cast_lanes(
                    frame,
                    &mut next_address,
                    *abi,
                    layout,
                    &input[usize::from(abi.pad_i32)..],
                )?;
                let value = if value_is_address(graph, ty) {
                    address
                } else {
                    self.normalize(ty, read_memory(frame, address, layout.size).bits)
                };
                frame.fixed_values.insert(parameter, Scalar::new(value));
            } else if let kfrc_body::PassMode::Vector { bytes } = mode {
                if *bytes != 16 {
                    return Err(InterpretError::Malformed(
                        "unsupported SIMD ABI width".to_owned(),
                    ));
                }
                let ty = graph.rust_type(parameter).ok_or_else(|| {
                    InterpretError::Malformed("SIMD parameter lacks type".to_owned())
                })?;
                let layout = graph.oracle.size_align(ty).ok_or_else(|| {
                    InterpretError::Malformed("SIMD parameter lacks layout".to_owned())
                })?;
                if layout.size != 16 {
                    return Err(InterpretError::Malformed(
                        "SIMD parameter is not 16 bytes".to_owned(),
                    ));
                }
                let input = arguments
                    .get(argument_index..argument_index + 2)
                    .ok_or_else(|| {
                        InterpretError::Malformed("too few SIMD parameter words".to_owned())
                    })?;
                argument_index += 2;
                next_address = next_address.next_multiple_of(layout.align.max(1));
                let address = next_address;
                next_address += 32;
                for (word, bits) in input.iter().enumerate() {
                    for (byte, bits) in bits.to_le_bytes().iter().enumerate() {
                        frame
                            .memory
                            .insert(address + (word * 8 + byte) as u64, *bits);
                    }
                }
                frame.fixed_values.insert(parameter, Scalar::new(address));
            } else if *mode == kfrc_body::PassMode::Pair {
                let ty = graph.rust_type(parameter).ok_or_else(|| {
                    InterpretError::Malformed("pair parameter lacks type".to_owned())
                })?;
                let lanes = graph.oracle.abi_pairs.get(&ty).ok_or_else(|| {
                    InterpretError::Malformed("pair parameter lacks ABI lane layout".to_owned())
                })?;
                let layout = graph.oracle.size_align(ty).ok_or_else(|| {
                    InterpretError::Malformed("pair parameter lacks layout".to_owned())
                })?;
                next_address = next_address.next_multiple_of(layout.align.max(1));
                let address = next_address;
                next_address += layout.size.max(1) + 16;
                for lane in lanes {
                    let size = graph
                        .oracle
                        .size_align(lane.ty)
                        .ok_or_else(|| {
                            InterpretError::Malformed("pair lane lacks layout".to_owned())
                        })?
                        .size;
                    let value = *arguments.get(argument_index).ok_or_else(|| {
                        InterpretError::Malformed("too few pair arguments".to_owned())
                    })?;
                    argument_index += 1;
                    for byte in 0..size {
                        frame.memory.insert(
                            address + lane.offset + byte,
                            value.to_le_bytes()[byte as usize],
                        );
                    }
                }
                frame.fixed_values.insert(parameter, Scalar::new(address));
            } else if *mode == kfrc_body::PassMode::Direct
                && graph
                    .rust_type(parameter)
                    .is_some_and(|ty| value_is_address(graph, ty))
            {
                let ty = graph.rust_type(parameter).expect("checked");
                let layout = graph.oracle.size_align(ty).ok_or_else(|| {
                    InterpretError::Malformed("direct aggregate lacks layout".to_owned())
                })?;
                if layout.size > 8 {
                    return Err(InterpretError::Malformed(
                        "direct aggregate exceeds one lane".to_owned(),
                    ));
                }
                let value = *arguments
                    .get(argument_index)
                    .ok_or_else(|| InterpretError::Malformed("too few arguments".to_owned()))?;
                argument_index += 1;
                let address = allocate_scalar(frame, &mut next_address, layout, value);
                frame.fixed_values.insert(parameter, Scalar::new(address));
            } else if *mode != kfrc_body::PassMode::Ignore {
                argument_index += 1;
            }
        }
        let mut control = graph.find_projection(graph.start, 0).ok_or_else(|| {
            InterpretError::Malformed("start has no control projection".to_owned())
        })?;
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
            // Fixed value nodes (division) evaluate when control passes them.
            frame.active_control = control;
            for user in &users {
                if is_anchored_value(graph.op(*user)) {
                    frame.fixed_values.remove(user);
                }
            }
            for user in &users {
                if is_anchored_value(graph.op(*user)) {
                    self.evaluate_pure(*user, frame, false)?;
                }
            }
            let next = users.iter().copied().find(|user| {
                matches!(
                    graph.op(*user),
                    Opcode::If
                        | Opcode::Jump
                        | Opcode::Return
                        | Opcode::Unreachable
                        | Opcode::Call
                        | Opcode::CallIndirect
                        | Opcode::Opaque
                        | Opcode::SlotLive
                        | Opcode::SlotDead
                        | Opcode::ThreadLocalAddress
                        | Opcode::Load
                        | Opcode::Store
                )
            });
            let Some(next) = next else {
                return Err(InterpretError::Malformed(format!(
                    "control {control} has no successor"
                )));
            };
            match graph.op(next) {
                Opcode::If => {
                    let condition = self.evaluate(graph.input(next, 0), frame)?;
                    if condition.poison {
                        return Err(InterpretError::PoisonObserved(next));
                    }
                    let index = if condition.bits & 1 == 1 { 0 } else { 1 };
                    control = graph.find_projection(next, index).ok_or_else(|| {
                        InterpretError::Malformed(format!("{next} lacks projection {index}"))
                    })?;
                }
                Opcode::Jump => {
                    let (region, slot) = *self.region_of_jump.get(&next).ok_or_else(|| {
                        InterpretError::Malformed(format!("{next} jumps nowhere"))
                    })?;
                    let phis = self
                        .phis_of_region
                        .get(&region)
                        .cloned()
                        .unwrap_or_default();
                    let mut values = Vec::with_capacity(phis.len());
                    for phi in &phis {
                        let operand = graph.input(*phi, slot + 1);
                        let value = if matches!(graph.value_type(*phi), ValueType::Effect) {
                            Scalar::new(0)
                        } else {
                            self.evaluate(operand, frame)?
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
                    let value = self.evaluate(inputs[1], frame)?;
                    if value.poison {
                        return Err(InterpretError::PoisonObserved(next));
                    }
                    let typed_boundary =
                        self.observe_layout_value(inputs[1], value, frame, next)?;
                    if let kfrc_body::PassMode::Cast(abi) = graph.abi.ret {
                        let ty = graph.rust_type(inputs[1]).ok_or_else(|| {
                            InterpretError::Malformed("cast return lacks type".to_owned())
                        })?;
                        let layout = graph.oracle.size_align(ty).ok_or_else(|| {
                            InterpretError::Malformed("cast return lacks layout".to_owned())
                        })?;
                        let address = if value_is_address(graph, ty) {
                            value.bits
                        } else {
                            allocate_scalar(frame, &mut next_address, layout, value.bits)
                        };
                        let lanes = read_cast_lanes(frame, abi, address, typed_boundary)?;
                        return match lanes.as_slice() {
                            [value] => Ok(Outcome::Return(*value)),
                            [first, second] => Ok(Outcome::ReturnPair(*first, *second)),
                            _ => Err(InterpretError::Malformed(
                                "cast return must have one or two lanes".to_owned(),
                            )),
                        };
                    }
                    if let kfrc_body::PassMode::Vector { bytes } = graph.abi.ret {
                        if bytes != 16 {
                            return Err(InterpretError::Malformed(
                                "unsupported SIMD return width".to_owned(),
                            ));
                        }
                        let first = read_memory(frame, value.bits, 8);
                        let second = read_memory(frame, value.bits + 8, 8);
                        if first.poison || second.poison {
                            return Err(InterpretError::PoisonObserved(next));
                        }
                        return Ok(Outcome::ReturnPair(first.bits, second.bits));
                    }
                    if graph.abi.ret == kfrc_body::PassMode::Pair {
                        let ty = graph.rust_type(inputs[1]).ok_or_else(|| {
                            InterpretError::Malformed("pair return lacks type".to_owned())
                        })?;
                        let lanes = graph.oracle.abi_pairs.get(&ty).ok_or_else(|| {
                            InterpretError::Malformed(
                                "pair return lacks ABI lane layout".to_owned(),
                            )
                        })?;
                        let mut result = [0u64; 2];
                        for (index, lane) in lanes.iter().enumerate() {
                            let size = graph
                                .oracle
                                .size_align(lane.ty)
                                .ok_or_else(|| {
                                    InterpretError::Malformed("pair lane lacks layout".to_owned())
                                })?
                                .size;
                            let scalar = read_memory(frame, value.bits + lane.offset, size);
                            if scalar.poison && !typed_boundary {
                                return Err(InterpretError::PoisonObserved(next));
                            }
                            result[index] = self.normalize(lane.ty, scalar.bits);
                        }
                        return Ok(Outcome::ReturnPair(result[0], result[1]));
                    }
                    if graph.abi.ret == kfrc_body::PassMode::Direct {
                        let ty = graph.rust_type(inputs[1]).ok_or_else(|| {
                            InterpretError::Malformed("return lacks type".to_owned())
                        })?;
                        if value_is_address(graph, ty) {
                            let layout = graph.oracle.size_align(ty).ok_or_else(|| {
                                InterpretError::Malformed("return lacks layout".to_owned())
                            })?;
                            let scalar = read_memory(frame, value.bits, layout.size);
                            if scalar.poison && !typed_boundary {
                                return Err(InterpretError::PoisonObserved(next));
                            }
                            return Ok(Outcome::Return(scalar.bits));
                        }
                    }
                    return Ok(Outcome::Return(value.bits));
                }
                Opcode::Unreachable => return Ok(Outcome::Unreachable),
                Opcode::ThreadLocalAddress => {
                    let (symbol, addend) = &graph.symbols[graph.payload(next) as usize];
                    if *addend != 0 {
                        return Err(InterpretError::Malformed(
                            "TLS address has an addend".to_owned(),
                        ));
                    }
                    let address = thread.address(symbol).ok_or_else(|| {
                        InterpretError::Malformed(format!(
                            "uninitialized external TLS symbol {symbol}"
                        ))
                    })?;
                    if let Some(result) = graph.find_projection(next, 2) {
                        frame.fixed_values.insert(result, Scalar::new(address));
                    }
                    frame.cache.clear();
                    control = graph.find_projection(next, 0).ok_or_else(|| {
                        InterpretError::Malformed("TLS address lacks continuation".to_owned())
                    })?;
                }
                Opcode::SlotLive | Opcode::SlotDead => {
                    let pointer = self.evaluate(graph.input(next, 1), frame)?;
                    if pointer.poison {
                        return Err(InterpretError::PoisonObserved(next));
                    }
                    let slot = frame
                        .slots
                        .iter()
                        .position(|address| *address == pointer.bits)
                        .ok_or_else(|| {
                            InterpretError::Malformed(
                                "lifetime marker has no stack allocation".into(),
                            )
                        })?;
                    let end = pointer.bits + graph.stack_slots[slot].size;
                    frame
                        .memory
                        .retain(|address, _| *address < pointer.bits || *address >= end);
                    if graph.op(next) == Opcode::SlotDead {
                        frame.dead_slots.insert(slot);
                    } else {
                        frame.dead_slots.remove(&slot);
                    }
                    frame.cache.clear();
                    control = graph.find_projection(next, 0).ok_or_else(|| {
                        InterpretError::Malformed("lifetime marker lacks continuation".into())
                    })?;
                }
                Opcode::Opaque => {
                    let value = self.evaluate(graph.input(next, 1), frame)?;
                    if let Some(result) = graph.find_projection(next, 2) {
                        frame.fixed_values.insert(result, value);
                    }
                    frame.cache.clear();
                    control = graph.find_projection(next, 0).ok_or_else(|| {
                        InterpretError::Malformed("opaque lacks continuation".to_owned())
                    })?;
                }
                Opcode::Load | Opcode::Store => {
                    let address = self.evaluate(graph.input(next, 1), frame)?;
                    if address.poison {
                        return Err(InterpretError::PoisonObserved(next));
                    }
                    if frame.dead_slots.iter().any(|slot| {
                        let base = frame.slots[*slot];
                        address.bits >= base
                            && address.bits < base + graph.stack_slots[*slot].size.max(1)
                    }) {
                        return Err(InterpretError::UndefinedBehaviour(
                            "access after SlotDead".into(),
                        ));
                    }
                    if graph.op(next) == Opcode::Load {
                        if let Some(value_node) = graph.find_projection(next, 2) {
                            let ty = graph.rust_type(value_node).ok_or_else(|| {
                                InterpretError::Malformed("load lacks value type".to_owned())
                            })?;
                            let size = graph
                                .oracle
                                .size_align(ty)
                                .ok_or_else(|| {
                                    InterpretError::Malformed("load lacks layout".to_owned())
                                })?
                                .size;
                            if size > 8 {
                                return Err(InterpretError::Malformed("aggregate load".to_owned()));
                            }
                            let mut bytes = [0u8; 8];
                            let mut poison = false;
                            for offset in 0..size {
                                match frame.memory.get(&(address.bits + offset)) {
                                    Some(byte) => bytes[offset as usize] = *byte,
                                    None => poison = true,
                                }
                            }
                            frame.fixed_values.insert(
                                value_node,
                                Scalar {
                                    bits: self.normalize(ty, u64::from_le_bytes(bytes)),
                                    poison,
                                },
                            );
                        }
                    } else {
                        let value_node = graph.input(next, 2);
                        let value = self.evaluate(value_node, frame)?;
                        let ty = graph.rust_type(value_node).ok_or_else(|| {
                            InterpretError::Malformed("store lacks value type".to_owned())
                        })?;
                        let size = graph
                            .oracle
                            .size_align(ty)
                            .ok_or_else(|| {
                                InterpretError::Malformed("store lacks layout".to_owned())
                            })?
                            .size;
                        if size > 8 {
                            return Err(InterpretError::Malformed("aggregate store".to_owned()));
                        }
                        for offset in 0..size {
                            if value.poison {
                                frame.memory.remove(&(address.bits + offset));
                            } else {
                                frame.memory.insert(
                                    address.bits + offset,
                                    value.bits.to_le_bytes()[offset as usize],
                                );
                            }
                        }
                    }
                    frame.cache.clear();
                    control = graph.find_projection(next, 0).ok_or_else(|| {
                        InterpretError::Malformed(format!("{next} lacks control"))
                    })?;
                }
                Opcode::Call | Opcode::CallIndirect => {
                    let (symbol, abi, argument_start) = if graph.op(next) == Opcode::Call {
                        let callee = &graph.callees[graph.payload(next) as usize];
                        (callee.symbol.clone(), &callee.abi, 1)
                    } else {
                        let target = self.evaluate(graph.input(next, 1), frame)?;
                        if target.poison {
                            return Err(InterpretError::PoisonObserved(next));
                        }
                        let symbol = frame
                            .symbols
                            .iter()
                            .find(|(_, address)| **address == target.bits)
                            .map(|(symbol, _)| symbol.clone())
                            .ok_or_else(|| {
                                InterpretError::UnknownCallee(format!("address {}", target.bits))
                            })?;
                        (symbol, &graph.call_abis[graph.payload(next) as usize], 2)
                    };
                    let mut arguments = Vec::new();
                    for (input, mode) in graph.inputs(next)[argument_start..].iter().zip(&abi.args)
                    {
                        if *mode == kfrc_body::PassMode::Ignore {
                            continue;
                        }
                        let value = self.evaluate(*input, frame)?;
                        if value.poison {
                            return Err(InterpretError::PoisonObserved(next));
                        }
                        let typed_boundary =
                            self.observe_layout_value(*input, value, frame, next)?;
                        if let kfrc_body::PassMode::Cast(abi) = mode {
                            let ty = graph.rust_type(*input).ok_or_else(|| {
                                InterpretError::Malformed("cast argument lacks type".to_owned())
                            })?;
                            let layout = graph.oracle.size_align(ty).ok_or_else(|| {
                                InterpretError::Malformed("cast argument lacks layout".to_owned())
                            })?;
                            let address = if value_is_address(graph, ty) {
                                value.bits
                            } else {
                                allocate_scalar(frame, &mut next_address, layout, value.bits)
                            };
                            if abi.pad_i32 {
                                arguments.push(0);
                            }
                            arguments.extend(read_cast_lanes(
                                frame,
                                *abi,
                                address,
                                typed_boundary,
                            )?);
                        } else if let kfrc_body::PassMode::Vector { bytes } = mode {
                            if *bytes != 16 {
                                return Err(InterpretError::Malformed(
                                    "unsupported SIMD argument width".to_owned(),
                                ));
                            }
                            for offset in [0, 8] {
                                let word = read_memory(frame, value.bits + offset, 8);
                                if word.poison {
                                    return Err(InterpretError::PoisonObserved(next));
                                }
                                arguments.push(word.bits);
                            }
                        } else if *mode == kfrc_body::PassMode::Pair {
                            let ty = graph.rust_type(*input).ok_or_else(|| {
                                InterpretError::Malformed(
                                    "pair call argument lacks type".to_owned(),
                                )
                            })?;
                            let lanes = graph.oracle.abi_pairs.get(&ty).ok_or_else(|| {
                                InterpretError::Malformed(
                                    "pair argument lacks ABI lane layout".to_owned(),
                                )
                            })?;
                            for lane in lanes {
                                let size = graph
                                    .oracle
                                    .size_align(lane.ty)
                                    .ok_or_else(|| {
                                        InterpretError::Malformed(
                                            "pair lane lacks layout".to_owned(),
                                        )
                                    })?
                                    .size;
                                let scalar = read_memory(frame, value.bits + lane.offset, size);
                                if scalar.poison && !typed_boundary {
                                    return Err(InterpretError::PoisonObserved(next));
                                }
                                arguments.push(self.normalize(lane.ty, scalar.bits));
                            }
                        } else if *mode == kfrc_body::PassMode::Direct
                            && graph
                                .rust_type(*input)
                                .is_some_and(|ty| value_is_address(graph, ty))
                        {
                            let ty = graph.rust_type(*input).expect("checked");
                            let layout = graph.oracle.size_align(ty).ok_or_else(|| {
                                InterpretError::Malformed("argument lacks layout".to_owned())
                            })?;
                            let scalar = read_memory(frame, value.bits, layout.size);
                            if scalar.poison && !typed_boundary {
                                return Err(InterpretError::PoisonObserved(next));
                            }
                            arguments.push(scalar.bits);
                        } else {
                            arguments.push(value.bits);
                        }
                    }
                    let memory_result = match symbol.as_str() {
                        "memcpy" | "memmove" if arguments.len() == 3 => {
                            let destination = arguments[0];
                            let source = arguments[1];
                            let bytes = arguments[2];
                            if symbol == "memcpy"
                                && bytes > 0
                                && destination < source.saturating_add(bytes)
                                && source < destination.saturating_add(bytes)
                            {
                                return Err(InterpretError::UndefinedBehaviour(
                                    "overlapping memcpy".to_owned(),
                                ));
                            }
                            let mut values = Vec::with_capacity(bytes.min(1_000_000) as usize);
                            if bytes > 1_000_000 {
                                return Err(InterpretError::OutOfFuel);
                            }
                            for offset in 0..bytes {
                                values.push(frame.memory.get(&(source + offset)).copied());
                            }
                            for (offset, byte) in values.into_iter().enumerate() {
                                if let Some(byte) = byte {
                                    frame.memory.insert(destination + offset as u64, byte);
                                } else {
                                    frame.memory.remove(&(destination + offset as u64));
                                }
                            }
                            Some(destination)
                        }
                        "memset" if arguments.len() == 3 => {
                            let destination = arguments[0];
                            let byte = arguments[1] as u8;
                            let bytes = arguments[2];
                            if bytes > 1_000_000 {
                                return Err(InterpretError::OutOfFuel);
                            }
                            for offset in 0..bytes {
                                frame.memory.insert(destination + offset, byte);
                            }
                            Some(destination)
                        }
                        // A memcmp ordering is also a valid bcmp result.
                        "memcmp" | "bcmp" if arguments.len() == 3 => {
                            let mut ordering = 0i32;
                            if arguments[2] > 1_000_000 {
                                return Err(InterpretError::OutOfFuel);
                            }
                            for offset in 0..arguments[2] {
                                let left = frame
                                    .memory
                                    .get(&(arguments[0] + offset))
                                    .ok_or(InterpretError::PoisonObserved(next))?;
                                let right = frame
                                    .memory
                                    .get(&(arguments[1] + offset))
                                    .ok_or(InterpretError::PoisonObserved(next))?;
                                if left != right {
                                    ordering = i32::from(*left) - i32::from(*right);
                                    break;
                                }
                            }
                            Some(ordering as u64)
                        }
                        _ => None,
                    };
                    let result = if let Some(result) = memory_result {
                        result
                    } else if let kfrc_body::PassMode::Cast(cast) = abi.ret {
                        let count = cast.lanes.iter().flatten().count();
                        let lanes = if count == 1 {
                            vec![calls.call(&symbol, &arguments)?]
                        } else if count == 2 {
                            let (a, b) = calls.call_pair(&symbol, &arguments)?;
                            vec![a, b]
                        } else {
                            return Err(InterpretError::Malformed(
                                "cast result must have one or two lanes".to_owned(),
                            ));
                        };
                        if let Some(result) = graph.find_projection(next, 2) {
                            let ty = graph.rust_type(result).ok_or_else(|| {
                                InterpretError::Malformed("cast result lacks type".to_owned())
                            })?;
                            let layout = graph.oracle.size_align(ty).ok_or_else(|| {
                                InterpretError::Malformed("cast result lacks layout".to_owned())
                            })?;
                            let address =
                                write_cast_lanes(frame, &mut next_address, cast, layout, &lanes)?;
                            if value_is_address(graph, ty) {
                                address
                            } else {
                                self.normalize(ty, read_memory(frame, address, layout.size).bits)
                            }
                        } else {
                            0
                        }
                    } else if let kfrc_body::PassMode::Vector { bytes } = abi.ret {
                        if bytes != 16 {
                            return Err(InterpretError::Malformed(
                                "unsupported SIMD result width".to_owned(),
                            ));
                        }
                        let (first, second) = calls.call_pair(&symbol, &arguments)?;
                        if let Some(value) = graph.find_projection(next, 2) {
                            let ty = graph.rust_type(value).ok_or_else(|| {
                                InterpretError::Malformed("SIMD result lacks type".to_owned())
                            })?;
                            let layout = graph.oracle.size_align(ty).ok_or_else(|| {
                                InterpretError::Malformed("SIMD result lacks layout".to_owned())
                            })?;
                            if layout.size != 16 {
                                return Err(InterpretError::Malformed(
                                    "SIMD result is not 16 bytes".to_owned(),
                                ));
                            }
                            next_address = next_address.next_multiple_of(layout.align.max(1));
                            let address = next_address;
                            next_address += 32;
                            for (index, bits) in [first, second].into_iter().enumerate() {
                                for (byte, bits) in bits.to_le_bytes().into_iter().enumerate() {
                                    frame
                                        .memory
                                        .insert(address + (index * 8 + byte) as u64, bits);
                                }
                            }
                            address
                        } else {
                            0
                        }
                    } else if abi.ret == kfrc_body::PassMode::Pair {
                        let (first, second) = calls.call_pair(&symbol, &arguments)?;
                        if let Some(value) = graph.find_projection(next, 2) {
                            let ty = graph.rust_type(value).ok_or_else(|| {
                                InterpretError::Malformed("pair result lacks type".to_owned())
                            })?;
                            let lanes = graph.oracle.abi_pairs.get(&ty).ok_or_else(|| {
                                InterpretError::Malformed(
                                    "pair result lacks lane layout".to_owned(),
                                )
                            })?;
                            let layout = graph.oracle.size_align(ty).ok_or_else(|| {
                                InterpretError::Malformed("pair result lacks layout".to_owned())
                            })?;
                            next_address = next_address.next_multiple_of(layout.align.max(1));
                            let address = next_address;
                            next_address += layout.size.max(1) + 16;
                            for (lane, bits) in lanes.iter().zip([first, second]) {
                                let size = graph
                                    .oracle
                                    .size_align(lane.ty)
                                    .ok_or_else(|| {
                                        InterpretError::Malformed(
                                            "pair lane lacks layout".to_owned(),
                                        )
                                    })?
                                    .size;
                                for byte in 0..size {
                                    frame.memory.insert(
                                        address + lane.offset + byte,
                                        bits.to_le_bytes()[byte as usize],
                                    );
                                }
                            }
                            address
                        } else {
                            0
                        }
                    } else {
                        calls.call(&symbol, &arguments)?
                    };
                    if let Some(value) = graph.find_projection(next, 2) {
                        let ty = graph.rust_type(value).expect("call value is a Rust type");
                        let bits = if abi.ret == kfrc_body::PassMode::Pair
                            || matches!(
                                abi.ret,
                                kfrc_body::PassMode::Cast(_) | kfrc_body::PassMode::Vector { .. }
                            ) {
                            result
                        } else if abi.ret == kfrc_body::PassMode::Direct
                            && value_is_address(graph, ty)
                        {
                            let layout = graph.oracle.size_align(ty).ok_or_else(|| {
                                InterpretError::Malformed(
                                    "aggregate result lacks layout".to_owned(),
                                )
                            })?;
                            allocate_scalar(frame, &mut next_address, layout, result)
                        } else {
                            self.normalize(ty, result)
                        };
                        frame.fixed_values.insert(value, Scalar::new(bits));
                    }
                    frame.cache.clear();
                    control = graph.find_projection(next, 0).ok_or_else(|| {
                        InterpretError::Malformed(format!("{next} lacks a control projection"))
                    })?;
                }
                _ => unreachable!("filtered above"),
            }
        }
    }

    fn evaluate(&self, node: NodeId, frame: &mut Frame) -> Result<Scalar, InterpretError> {
        self.evaluate_pure(node, frame, false)
    }

    /// ABI padding and inactive enum storage are not source-language values.
    /// Check the active typed leaves once, then choose zero for uninitialized
    /// padding when exposing physical register lanes to the test call handler.
    fn observe_layout_value(
        &self,
        node: NodeId,
        scalar: Scalar,
        frame: &Frame,
        observer: NodeId,
    ) -> Result<bool, InterpretError> {
        if let ValueType::LayoutValue(ty) = self.graph.value_type(node) {
            let value = SemanticMemory::decode_snapshot(
                &self.graph.oracle,
                scalar.bits,
                *ty,
                &frame.memory,
            )?;
            if value.contains_poison() {
                return Err(InterpretError::PoisonObserved(observer));
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn evaluate_pure(
        &self,
        node: NodeId,
        frame: &mut Frame,
        executing_fixed: bool,
    ) -> Result<Scalar, InterpretError> {
        let graph = self.graph;
        if let Some(value) = frame
            .phi_values
            .get(&node)
            .or_else(|| frame.fixed_values.get(&node))
            && !executing_fixed
        {
            return Ok(*value);
        }
        if let Some(value) = frame.cache.get(&node) {
            return Ok(*value);
        }
        if !executing_fixed && is_anchored_value(graph.op(node)) {
            if graph.control(node) != frame.active_control {
                return Err(InterpretError::Malformed(format!(
                    "{node} read before its control anchor"
                )));
            }
            let value = self.evaluate_pure(node, frame, true)?;
            frame.fixed_values.insert(node, value);
            return Ok(value);
        }
        let op = graph.op(node);
        let ty = graph.rust_type(node);
        let value = match op {
            Opcode::Const => Scalar::new(self.normalize(
                ty.expect("constant has a Rust type"),
                graph.constant_value(node) as u64,
            )),
            Opcode::StackSlot => Scalar::new(frame.slots[graph.payload(node) as usize]),
            Opcode::SymbolAddress => {
                let (symbol, addend) = &graph.symbols[graph.payload(node) as usize];
                let address = frame
                    .symbols
                    .get(symbol)
                    .ok_or_else(|| InterpretError::Malformed(format!("unknown symbol {symbol}")))?;
                Scalar::new(address.wrapping_add_signed(*addend))
            }
            Opcode::Undefined => Scalar {
                bits: 0,
                poison: true,
            },
            Opcode::Projection => {
                let source = graph.input(node, 0);
                if graph.op(source) != Opcode::Start {
                    return Err(InterpretError::Malformed(format!(
                        "value {node} read before its call ran"
                    )));
                }
                let index = graph.payload(node) as usize - 2;
                let ty = ty.expect("parameter has a Rust type");
                let passed = graph.abi.args[..index]
                    .iter()
                    .map(|mode| match mode {
                        kfrc_body::PassMode::Ignore => 0,
                        kfrc_body::PassMode::Pair | kfrc_body::PassMode::Vector { bytes: 16 } => 2,
                        kfrc_body::PassMode::Cast(abi) => cast_argument_count(*abi),
                        _ => 1,
                    })
                    .sum::<usize>();
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
                return Err(InterpretError::Malformed(format!(
                    "phi {node} read before its region ran"
                )));
            }
            Opcode::DivSigned | Opcode::DivUnsigned | Opcode::RemSigned | Opcode::RemUnsigned
                if !executing_fixed =>
            {
                return Err(InterpretError::Malformed(format!(
                    "{node} read before control reached it"
                )));
            }
            _ => {
                let inputs = graph.inputs(node).to_vec();
                let mut values = Vec::with_capacity(inputs.len());
                for input in &inputs {
                    values.push(self.evaluate(*input, frame)?);
                }
                let ty = ty.ok_or_else(|| {
                    InterpretError::Malformed(format!("{node} has no value type"))
                })?;
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
        if width >= 64 {
            bits
        } else {
            bits & ((1u64 << width) - 1)
        }
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

    fn apply(
        &self,
        node: NodeId,
        op: Opcode,
        ty: TyId,
        inputs: &[NodeId],
        values: &[Scalar],
    ) -> Result<Scalar, InterpretError> {
        let input_poison = values.iter().any(|value| value.poison);
        let mut poison = input_poison;
        let result_ty = ty;
        let operand_ty = inputs
            .first()
            .and_then(|input| self.graph.rust_type(*input))
            .unwrap_or(ty);
        let (width, _) = self.width_and_sign(operand_ty);
        let a = values.first().map_or(0, |value| value.bits);
        let b = values.get(1).map_or(0, |value| value.bits);
        if op.integer_intrinsic_arity().is_some() {
            if !matches!(width, 8 | 16 | 32 | 64) {
                return Err(InterpretError::Malformed(
                    "unsupported integer intrinsic width".into(),
                ));
            }
            let (_, signed) = self.width_and_sign(operand_ty);
            let operands: Vec<_> = values.iter().map(|value| value.bits).collect();
            let (bits, contract_poison) = crate::intrinsic::evaluate(
                op,
                self.graph.payload(node),
                width as u8,
                signed,
                &operands,
            );
            return Ok(Scalar {
                bits: self.normalize(result_ty, bits),
                poison: input_poison || contract_poison,
            });
        }
        let sa = self.signed(operand_ty, a);
        let sb = self.signed(operand_ty, b);
        if crate::ArithmeticFlags::allowed(op).is_some() {
            if width > 64 {
                return Err(InterpretError::Malformed(
                    "arithmetic contract exceeds supported scalar width".into(),
                ));
            }
            poison |=
                crate::ArithmeticFlags(self.graph.payload(node)).violated(op, width as u8, a, b);
        }
        let shift_amount = |amount: u64| (amount % u64::from(width.max(1))) as u32;
        let bits = match op {
            Opcode::Add | Opcode::PtrOffset => a.wrapping_add(b),
            Opcode::MulOverflow => {
                let (_, signed) = self.width_and_sign(operand_ty);
                if signed {
                    let product = i128::from(sa) * i128::from(sb);
                    let min = -(1i128 << (width - 1));
                    let max = (1i128 << (width - 1)) - 1;
                    u64::from(product < min || product > max)
                } else {
                    let product = u128::from(a) * u128::from(b);
                    u64::from(product > (1u128 << width) - 1)
                }
            }
            Opcode::Sub => a.wrapping_sub(b),
            Opcode::Mul => a.wrapping_mul(b),
            Opcode::DivSigned | Opcode::RemSigned | Opcode::DivUnsigned | Opcode::RemUnsigned => {
                if input_poison {
                    return Err(InterpretError::PoisonObserved(node));
                }
                if b == 0
                    || (matches!(op, Opcode::DivSigned | Opcode::RemSigned)
                        && sb == -1
                        && sa == self.signed(operand_ty, 1u64 << (width - 1)))
                {
                    return Err(InterpretError::UndefinedBehaviour(format!(
                        "{node}: division by zero or overflow"
                    )));
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
            other => {
                return Err(InterpretError::Malformed(format!(
                    "{node}: {other:?} is not a pure value"
                )));
            }
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

fn read_memory(frame: &Frame, address: u64, size: u64) -> Scalar {
    if size > 8 {
        return Scalar {
            bits: 0,
            poison: true,
        };
    }
    let mut bytes = [0u8; 8];
    let mut poison = false;
    for offset in 0..size {
        match frame.memory.get(&(address + offset)) {
            Some(byte) => bytes[offset as usize] = *byte,
            None => poison = true,
        }
    }
    Scalar {
        bits: u64::from_le_bytes(bytes),
        poison,
    }
}

fn value_is_address(graph: &FunctionGraph, ty: TyId) -> bool {
    graph
        .oracle
        .size_align(ty)
        .is_some_and(|layout| layout.size > 8)
        || matches!(
            graph.oracle.shape(ty),
            TypeShape::Tuple(_) | TypeShape::Array { .. } | TypeShape::Adt { .. }
        )
}

fn allocate_scalar(
    frame: &mut Frame,
    next_address: &mut u64,
    layout: kfrc_body::SizeAlign,
    value: u64,
) -> u64 {
    *next_address = next_address.next_multiple_of(layout.align.max(1));
    let address = *next_address;
    *next_address += layout.size.max(1) + 16;
    for byte in 0..layout.size.min(8) {
        frame
            .memory
            .insert(address + byte, value.to_le_bytes()[byte as usize]);
    }
    address
}

fn cast_argument_count(abi: kfrc_body::CastAbi) -> usize {
    abi.lanes.iter().flatten().count() + usize::from(abi.pad_i32)
}

fn write_cast_lanes(
    frame: &mut Frame,
    next_address: &mut u64,
    abi: kfrc_body::CastAbi,
    layout: kfrc_body::SizeAlign,
    values: &[u64],
) -> Result<u64, InterpretError> {
    if abi.size != layout.size {
        return Err(InterpretError::Malformed(
            "cast ABI size differs from Rust layout".to_owned(),
        ));
    }
    *next_address = next_address.next_multiple_of(layout.align.max(1));
    let address = *next_address;
    *next_address += layout.size.max(1) + 16;
    if values.len() != abi.lanes.iter().flatten().count() {
        return Err(InterpretError::Malformed(
            "cast lane arity differs".to_owned(),
        ));
    }
    for (lane, value) in abi.lanes.iter().flatten().zip(values) {
        if lane.bytes == 0 || lane.bytes > 8 || lane.offset >= abi.size {
            return Err(InterpretError::Malformed(
                "invalid cast lane extent".to_owned(),
            ));
        }
        let bytes = u64::from(lane.bytes).min(abi.size - lane.offset);
        for byte in 0..bytes {
            frame.memory.insert(
                address + lane.offset + byte,
                value.to_le_bytes()[byte as usize],
            );
        }
    }
    Ok(address)
}

fn read_cast_lanes(
    frame: &Frame,
    abi: kfrc_body::CastAbi,
    address: u64,
    typed_boundary: bool,
) -> Result<Vec<u64>, InterpretError> {
    let mut values = Vec::new();
    for lane in abi.lanes.iter().flatten() {
        if lane.bytes == 0 || lane.bytes > 8 || lane.offset >= abi.size {
            return Err(InterpretError::Malformed(
                "invalid cast lane extent".to_owned(),
            ));
        }
        let bytes = u64::from(lane.bytes).min(abi.size - lane.offset);
        let value = read_memory(frame, address + lane.offset, bytes);
        if value.poison && !typed_boundary {
            return Err(InterpretError::UndefinedBehaviour(
                "uninitialized cast ABI lane".to_owned(),
            ));
        }
        let bits = if abi.extension == kfrc_body::CastExtension::Sign && lane.bytes < 8 {
            let shift = 64 - u32::from(lane.bytes) * 8;
            ((value.bits << shift) as i64 >> shift) as u64
        } else {
            value.bits
        };
        values.push(bits);
    }
    Ok(values)
}

/// An assumption is a source UB requirement, not a check with runtime failure
/// behavior. Invalid interpreter representations remain compiler/harness errors.
fn check_assumption(predicate: &Value, node: NodeId) -> Result<(), InterpretError> {
    match predicate {
        Value::Scalar(1) => Ok(()),
        Value::Scalar(0) | Value::Poison => Err(InterpretError::UndefinedBehaviour(format!(
            "Assume predicate is false or poison at {node}"
        ))),
        _ => Err(InterpretError::Malformed(format!(
            "Assume predicate is not a bool at {node}"
        ))),
    }
}

/// Witnesses have no runtime value. Their consumers validate the original enum
/// tag; payload poison is unrelated to the truth of a variant fact. Static
/// verification checks operand identity, proof scope and the consumer's placement.
fn check_variant_witness(
    graph: &FunctionGraph,
    consumer: NodeId,
    value: &Value,
) -> Result<(), InterpretError> {
    let Some(&witness) = graph.inputs(consumer).get(1) else {
        return Ok(());
    };
    if !matches!(value, Value::Variant { variant, .. } if *variant == graph.payload(witness)) {
        return Err(InterpretError::Malformed(format!(
            "VariantWitness {witness} failed at {consumer}"
        )));
    }
    Ok(())
}
