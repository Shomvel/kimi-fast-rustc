//! Calling-convention transport: signatures, parameters, calls and returns.
//!
//! The graph records rustc's `PassMode` per argument and result. A scalar
//! travels as a typed Cranelift value; an aggregate at Layout level is the
//! address of its storage, read into register lanes on the way out and
//! written to fresh frame storage on the way in. Cranelift's System V
//! implementation then assigns integer and vector registers in order and
//! places the rest on the stack, the same assignment LLVM makes for the
//! lanes rustc selected.

use cranelift_codegen::ir::{self, AbiParam, InstBuilder, Value, types};
use cranelift_codegen::isa::CallConv;
use kfrc_body::{
    CastAbi, CastExtension, CastLane, FunctionAbi, PassMode, TyId, TypeOracle, TypeShape,
};
use kfrc_son::{NodeId, Opcode, ValueType};

use super::memory::memory_flags;
use super::{Exporter, LowerError, Scalar};

/// Sixteen object bytes in one SSE register.
const VECTOR_LANE: ir::Type = types::I8X16;

fn cast_lanes(cast: CastAbi) -> Result<Vec<CastLane>, LowerError> {
    let lanes = cast.lanes.into_iter().flatten().collect::<Vec<_>>();
    if lanes.iter().any(|lane| !(1..=8).contains(&lane.bytes)) {
        return Err(LowerError::Unsupported(
            "Cast ABI lane wider than a general register".to_owned(),
        ));
    }
    if cast.extension != CastExtension::None && lanes.len() != 1 {
        return Err(LowerError::Unsupported(
            "Cast ABI extension on multiple lanes".to_owned(),
        ));
    }
    if cast.consecutive && lanes.len() > 1 {
        return Err(LowerError::Unsupported(
            "Cast ABI lanes that must stay consecutive".to_owned(),
        ));
    }
    Ok(lanes)
}

/// A scalar lane. The caller extends values narrower than 32 bits to the
/// whole register, so a callee compiled by any backend may read either width.
fn scalar_lane(scalar: Scalar) -> Option<AbiParam> {
    let lane = AbiParam::new(scalar.ty()?);
    Some(if scalar.bits >= 32 {
        lane
    } else if scalar.signed {
        lane.sext()
    } else {
        lane.uext()
    })
}

fn tuple_element(ty: &ValueType, index: usize) -> Option<TyId> {
    match ty {
        ValueType::Tuple(elements) => match elements.get(index) {
            Some(ValueType::Rust(ty) | ValueType::LayoutValue(ty)) => Some(*ty),
            _ => None,
        },
        _ => None,
    }
}

impl Exporter<'_, '_> {
    /// A transparent address view carries the same whole value, with no
    /// field read, byte reinterpretation or pointer arithmetic.
    fn storage_source(&self, mut node: NodeId) -> NodeId {
        let graph = self.graph;
        node = graph.resolve(node);
        while graph.op(node) == Opcode::PtrOffset {
            let offset = graph.resolve(graph.input(node, 1));
            if graph.op(offset) != Opcode::Const || graph.constant_value(offset) != 0 {
                break;
            }
            node = graph.resolve(graph.input(node, 0));
        }
        node
    }

    /// A same-typed result immediately returned by this block can be built
    /// in the caller's storage. No intervening effect observes a partial
    /// result. The result is a value-owned allocation, so its identity cannot
    /// be observed independently of its representation.
    fn forwards_call_result(&self, call: NodeId, result: NodeId) -> bool {
        let graph = self.graph;
        if graph.abi.ret != PassMode::Indirect {
            return false;
        }
        let chain = &self.schedule.blocks[self.current_block].chain;
        let [.., last_call, returned] = chain.as_slice() else {
            return false;
        };
        graph.op(*returned) == Opcode::Return
            && *last_call == call
            && graph.rust_type(result) == self.return_type()
            && graph
                .inputs(*returned)
                .get(1)
                .is_some_and(|&value| self.storage_source(value) == result)
    }

    /// The raw-integer lane of a directly passed aggregate of 1, 2, 4 or 8
    /// bytes.
    fn direct_aggregate_scalar(&self, ty: TyId) -> Result<Scalar, LowerError> {
        if matches!(self.graph.oracle.shape(ty), TypeShape::Float { .. }) {
            return Err(LowerError::Unsupported(
                "floating-point values in a calling convention".to_owned(),
            ));
        }
        let layout = self.layout_of(ty)?;
        match layout.size {
            1 | 2 | 4 | 8 => Ok(Scalar {
                bits: (layout.size * 8) as u8,
                signed: false,
            }),
            size => Err(LowerError::Unsupported(format!(
                "direct aggregate of {size} bytes"
            ))),
        }
    }

    fn pair_fields(&self, ty: TyId) -> Result<[kfrc_body::FieldLayout; 2], LowerError> {
        self.graph
            .oracle
            .abi_pairs
            .get(&ty)
            .copied()
            .ok_or_else(|| {
                LowerError::Unsupported(format!(
                    "missing ScalarPair layout for `{}`",
                    self.graph.oracle.describe(ty)
                ))
            })
    }

    fn push_lanes(
        &self,
        lanes: &mut Vec<AbiParam>,
        mode: &PassMode,
        ty: Option<TyId>,
    ) -> Result<(), LowerError> {
        let known =
            || ty.ok_or_else(|| LowerError::Malformed("ABI value has no Rust type".to_owned()));
        match mode {
            PassMode::Ignore => {}
            PassMode::Direct => {
                let scalar = match ty {
                    // An unread parameter still occupies its register.
                    None => Scalar::ADDRESS,
                    Some(ty) => match self.scalar_of_type(ty) {
                        Ok(scalar) => scalar,
                        Err(_) => self.direct_aggregate_scalar(ty)?,
                    },
                };
                // A direct value without bits occupies no lane.
                lanes.extend(scalar_lane(scalar));
            }
            PassMode::Pair => {
                for field in self.pair_fields(known()?)? {
                    let scalar = self.scalar_of_type(field.ty)?;
                    lanes.push(scalar_lane(scalar).ok_or_else(|| {
                        LowerError::Unsupported("ScalarPair field without bits".to_owned())
                    })?);
                }
            }
            PassMode::Cast(cast) => {
                if cast.pad_i32 {
                    lanes.push(AbiParam::new(types::I64));
                }
                for _ in cast_lanes(*cast)? {
                    lanes.push(AbiParam::new(types::I64));
                }
            }
            PassMode::Vector { bytes: 16 } => lanes.push(AbiParam::new(VECTOR_LANE)),
            PassMode::Vector { bytes } => {
                return Err(LowerError::Unsupported(format!("{bytes}-byte vector ABI")));
            }
            PassMode::Indirect => lanes.push(AbiParam::new(types::I64)),
            PassMode::IndirectOnStack => {
                return Err(LowerError::Unsupported(
                    "by-value memory argument".to_owned(),
                ));
            }
        }
        Ok(())
    }

    /// The System V signature of a function with `abi`. An indirect result is
    /// a leading pointer parameter that the callee also returns.
    fn signature(
        &self,
        abi: &FunctionAbi,
        argument_types: &[Option<TyId>],
        result_type: Option<TyId>,
    ) -> Result<ir::Signature, LowerError> {
        let mut signature = ir::Signature::new(CallConv::SystemV);
        if abi.ret == PassMode::Indirect {
            signature.params.push(AbiParam::new(types::I64));
            signature.returns.push(AbiParam::new(types::I64));
        } else {
            self.push_lanes(&mut signature.returns, &abi.ret, result_type)?;
            if signature.returns.len() > 2 {
                return Err(LowerError::Unsupported(
                    "return value in more than two registers".to_owned(),
                ));
            }
        }
        if argument_types.len() != abi.args.len() {
            return Err(LowerError::Malformed(
                "argument count differs from the ABI".to_owned(),
            ));
        }
        for (mode, ty) in abi.args.iter().zip(argument_types) {
            self.push_lanes(&mut signature.params, mode, *ty)?;
        }
        Ok(signature)
    }

    pub(super) fn function_signature(&self) -> Result<ir::Signature, LowerError> {
        let graph = self.graph;
        let start = graph.value_type(graph.start);
        let argument_types: Vec<Option<TyId>> = (0..graph.abi.args.len())
            .map(|index| tuple_element(start, 2 + index))
            .collect();
        if self.returns() {
            return self.signature(&graph.abi, &argument_types, self.return_type());
        }
        // No path reaches the caller, so no result lane is ever written. An
        // indirect result still occupies the first argument register.
        let diverging = FunctionAbi {
            args: graph.abi.args.clone(),
            ret: if graph.abi.ret == PassMode::Indirect {
                PassMode::Indirect
            } else {
                PassMode::Ignore
            },
            c_variadic: graph.abi.c_variadic,
        };
        let mut signature = self.signature(&diverging, &argument_types, None)?;
        signature.returns.clear();
        Ok(signature)
    }

    fn returns(&self) -> bool {
        self.schedule.blocks.iter().any(|block| {
            block
                .chain
                .last()
                .is_some_and(|&last| self.graph.op(last) == Opcode::Return)
        })
    }

    /// The Rust type every `Return` of this function carries.
    fn return_type(&self) -> Option<TyId> {
        let graph = self.graph;
        self.schedule.blocks.iter().find_map(|block| {
            let &last = block.chain.last()?;
            if graph.op(last) != Opcode::Return {
                return None;
            }
            graph.rust_type(*graph.inputs(last).get(1)?)
        })
    }

    /// The register lanes that carry the aggregate or scalar `node` under a
    /// Cast mode, lowest offset first.
    fn load_cast(&mut self, node: NodeId, cast: CastAbi) -> Result<Vec<Value>, LowerError> {
        let ty = self
            .graph
            .rust_type(node)
            .ok_or_else(|| LowerError::Malformed("Cast operand lacks Rust type".to_owned()))?;
        if self.promotion.place(self.graph.resolve(node)).is_some() {
            // Promoted storage has exactly the lanes of this transfer.
            let mut lanes = Vec::new();
            for lane in cast_lanes(cast)? {
                let count = u64::from(lane.bytes).min(cast.size.saturating_sub(lane.offset)) as u8;
                let ty = super::integer_type(count * 8).ok_or_else(|| {
                    LowerError::Malformed("promoted Cast lane of irregular width".to_owned())
                })?;
                let value = self.read_place(node, lane.offset, ty)?;
                lanes.push(self.coerce(value, Scalar::ADDRESS, 64));
            }
            return Ok(lanes);
        }
        let address = match self.scalar_of_type(ty) {
            Ok(scalar) => {
                let value = self.operand(node, 64)?;
                let address = self.new_stack_storage_of(ty)?;
                self.store_raw_bytes(address, 0, scalar.bits / 8, value);
                address
            }
            Err(_) => self.address(node)?,
        };
        let mut lanes = Vec::new();
        for lane in cast_lanes(cast)? {
            let count = u64::from(lane.bytes).min(cast.size.saturating_sub(lane.offset)) as u8;
            let value = self.load_raw_bytes(address, lane.offset, count);
            lanes.push(match cast.extension {
                CastExtension::None => value,
                extension => {
                    self.extend_low_bytes(value, lane.bytes, extension == CastExtension::Sign)
                }
            });
        }
        Ok(lanes)
    }

    /// Extends the low `bytes` bytes of a 64-bit value to the whole register.
    fn extend_low_bytes(&mut self, value: Value, bytes: u8, signed: bool) -> Value {
        if bytes >= 8 {
            return value;
        }
        let shift = i64::from(64 - u32::from(bytes) * 8);
        let shifted = self.builder.ins().ishl_imm_u(value, shift);
        if signed {
            self.builder.ins().sshr_imm_u(shifted, shift)
        } else {
            self.builder.ins().ushr_imm_u(shifted, shift)
        }
    }

    /// Binds a value that arrived in Cast lanes: an aggregate gets frame
    /// storage, a scalar is reassembled from its bytes.
    fn receive_cast(
        &mut self,
        result: NodeId,
        cast: CastAbi,
        values: &[Value],
    ) -> Result<(), LowerError> {
        let ty = self
            .graph
            .rust_type(result)
            .ok_or_else(|| LowerError::Malformed("Cast result lacks Rust type".to_owned()))?;
        let lanes = cast_lanes(cast)?;
        if lanes.len() != values.len() {
            return Err(LowerError::Malformed(
                "Cast lane/value count mismatch".to_owned(),
            ));
        }
        if let Some(storage) = self.promotion.received(result) {
            for (lane, value) in lanes.into_iter().zip(values) {
                let count = u64::from(lane.bytes).min(cast.size.saturating_sub(lane.offset)) as u8;
                super::integer_type(count * 8).ok_or_else(|| {
                    LowerError::Malformed("promoted Cast lane of irregular width".to_owned())
                })?;
                let value = self.coerce(*value, Scalar::ADDRESS, count * 8);
                self.write_storage(storage, lane.offset, value)?;
            }
            return Ok(());
        }
        let address = self.new_stack_storage_of(ty)?;
        for (lane, value) in lanes.into_iter().zip(values) {
            let count = u64::from(lane.bytes).min(cast.size.saturating_sub(lane.offset)) as u8;
            self.store_raw_bytes(address, lane.offset, count, *value);
        }
        let value = match self.scalar_of_type(ty) {
            Ok(scalar) => match scalar.ty() {
                Some(ty) => self.builder.ins().load(ty, memory_flags(), address, 0),
                None => return Ok(()),
            },
            Err(_) => address,
        };
        self.define(result, value);
        Ok(())
    }

    /// Binds an aggregate whose scalar parts arrived in registers: lane
    /// variables of promoted storage, or fresh frame storage.
    fn arrive(
        &mut self,
        result: NodeId,
        ty: TyId,
        parts: &[(u64, Value)],
    ) -> Result<(), LowerError> {
        if let Some(storage) = self.promotion.received(result) {
            for &(offset, value) in parts {
                self.write_storage(storage, offset, value)?;
            }
            return Ok(());
        }
        let address = self.new_stack_storage_of(ty)?;
        for &(offset, value) in parts {
            self.store_at(value, address, offset);
        }
        self.define(result, address);
        Ok(())
    }

    /// Binds a value that arrived in the lanes of `mode`. `lanes` yields the
    /// Cranelift values in signature order.
    fn receive(
        &mut self,
        result: Option<NodeId>,
        ty: Option<TyId>,
        mode: &PassMode,
        lanes: &mut impl Iterator<Item = Value>,
    ) -> Result<(), LowerError> {
        let mut next = || {
            lanes
                .next()
                .ok_or_else(|| LowerError::Malformed("missing ABI lane".to_owned()))
        };
        match mode {
            PassMode::Ignore => {}
            PassMode::Direct => {
                let scalar = match ty {
                    None => Some(Scalar::ADDRESS),
                    Some(ty) => self.scalar_of_type(ty).ok(),
                };
                match (scalar, result) {
                    // A value without bits has no lane.
                    (Some(scalar), _) if scalar.bits == 0 => {}
                    (Some(_), Some(result)) => {
                        let value = next()?;
                        self.define(result, value);
                    }
                    (Some(_), None) => {
                        next()?;
                    }
                    (None, Some(result)) => {
                        let value = next()?;
                        self.arrive(result, ty.expect("typed result"), &[(0, value)])?;
                    }
                    (None, None) => {
                        next()?;
                    }
                }
            }
            PassMode::Pair => {
                let (first, second) = (next()?, next()?);
                if let (Some(result), Some(ty)) = (result, ty) {
                    let fields = self.pair_fields(ty)?;
                    self.arrive(
                        result,
                        ty,
                        &[(fields[0].offset, first), (fields[1].offset, second)],
                    )?;
                }
            }
            PassMode::Cast(cast) => {
                if cast.pad_i32 {
                    next()?;
                }
                let values = cast_lanes(*cast)?
                    .iter()
                    .map(|_| next())
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some(result) = result {
                    self.receive_cast(result, *cast, &values)?;
                }
            }
            PassMode::Vector { .. } => {
                let value = next()?;
                if let (Some(result), Some(ty)) = (result, ty) {
                    let address = self.new_stack_storage_of(ty)?;
                    self.builder.ins().store(memory_flags(), value, address, 0);
                    self.define(result, address);
                }
            }
            PassMode::Indirect => {
                let value = next()?;
                if let Some(result) = result {
                    self.define(result, value);
                }
            }
            PassMode::IndirectOnStack => {
                return Err(LowerError::Unsupported(
                    "by-value memory argument".to_owned(),
                ));
            }
        }
        Ok(())
    }

    /// Appends the lanes that carry `node` under `mode`.
    fn pass(
        &mut self,
        lanes: &mut Vec<Value>,
        node: NodeId,
        mode: &PassMode,
    ) -> Result<(), LowerError> {
        let ty = self
            .graph
            .rust_type(node)
            .ok_or_else(|| LowerError::Malformed("ABI value has no Rust type".to_owned()))?;
        match mode {
            PassMode::Ignore => {}
            PassMode::Direct => match self.scalar_of_type(ty) {
                Ok(scalar) => {
                    if scalar.bits != 0 {
                        lanes.push(self.operand(node, scalar.bits)?);
                    }
                }
                Err(_) => {
                    let scalar = self.direct_aggregate_scalar(ty)?;
                    let lane = scalar.ty().expect("direct aggregates have bits");
                    lanes.push(self.read_place(node, 0, lane)?);
                }
            },
            PassMode::Pair => {
                for field in self.pair_fields(ty)? {
                    let lane = self.scalar_of_type(field.ty)?.ty().ok_or_else(|| {
                        LowerError::Unsupported("ScalarPair field without bits".to_owned())
                    })?;
                    lanes.push(self.read_place(node, field.offset, lane)?);
                }
            }
            PassMode::Cast(cast) => {
                if cast.pad_i32 {
                    lanes.push(self.builder.ins().iconst(types::I64, 0));
                }
                lanes.extend(self.load_cast(node, *cast)?);
            }
            PassMode::Vector { .. } => {
                let address = self.address(node)?;
                lanes.push(
                    self.builder
                        .ins()
                        .load(VECTOR_LANE, memory_flags(), address, 0),
                );
            }
            // Layout lowering snapshots a by-value aggregate before the call;
            // the operand is that private address.
            PassMode::Indirect => lanes.push(self.address(node)?),
            PassMode::IndirectOnStack => {
                return Err(LowerError::Unsupported(
                    "by-value memory argument".to_owned(),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn receive_parameters(&mut self) -> Result<(), LowerError> {
        let graph = self.graph;
        let entry = self
            .builder
            .current_block()
            .expect("the entry block is open");
        let mut lanes = self.builder.block_params(entry).to_vec().into_iter();
        if graph.abi.ret == PassMode::Indirect {
            self.return_pointer = Some(lanes.next().ok_or_else(|| {
                LowerError::Malformed("missing hidden return pointer".to_owned())
            })?);
        }
        let start = graph.value_type(graph.start);
        for (index, mode) in graph.abi.args.iter().enumerate() {
            let parameter = self.projection(graph.start, 2 + index as u32);
            let ty = tuple_element(start, 2 + index);
            self.receive(parameter, ty, mode, &mut lanes)?;
        }
        Ok(())
    }

    pub(super) fn emit_call(&mut self, node: NodeId) -> Result<(), LowerError> {
        if self.emit_lane_copy(node)?
            || self.emit_small_copy(node)?
            || self.emit_small_equality(node)?
        {
            return Ok(());
        }
        let graph = self.graph;
        let (abi, first_argument) = if graph.op(node) == Opcode::Call {
            (&graph.callees[graph.payload(node) as usize].abi, 1)
        } else {
            (&graph.call_abis[graph.payload(node) as usize], 2)
        };
        let arguments = &graph.inputs(node)[first_argument..];
        let argument_types: Vec<Option<TyId>> = arguments
            .iter()
            .map(|argument| graph.rust_type(*argument))
            .collect();
        let result = self.projection(node, 2);
        let result_type = tuple_element(graph.value_type(node), 2);
        let signature = self.signature(abi, &argument_types, result_type)?;
        let signature = self.signature_reference(signature);

        let mut lanes = Vec::new();
        let returned_storage = if abi.ret == PassMode::Indirect {
            let ty = result_type.ok_or_else(|| {
                LowerError::Malformed("indirect call has no return type".to_owned())
            })?;
            let address = if result.is_some_and(|result| self.forwards_call_result(node, result)) {
                self.return_pointer
                    .ok_or_else(|| LowerError::Malformed("missing return storage".into()))?
            } else {
                self.new_stack_storage_of(ty)?
            };
            lanes.push(address);
            Some(address)
        } else {
            None
        };
        for (argument, mode) in arguments.iter().zip(&abi.args) {
            self.pass(&mut lanes, *argument, mode)?;
        }
        let call = if graph.op(node) == Opcode::Call {
            let symbol = &graph.callees[graph.payload(node) as usize].symbol;
            let function = self.function_reference(symbol, signature);
            self.builder.ins().call(function, &lanes)
        } else {
            let target = self.address(graph.input(node, 1))?;
            self.builder.ins().call_indirect(signature, target, &lanes)
        };
        match returned_storage {
            Some(address) => {
                if let Some(result) = result {
                    self.define(result, address);
                }
            }
            None => {
                let mut results = self.builder.inst_results(call).to_vec().into_iter();
                self.receive(result, result_type, &abi.ret, &mut results)?;
            }
        }
        self.continue_in_new_block();
        Ok(())
    }

    /// Writes the aggregate at the address node `source` to the caller's
    /// result storage: the lanes of promoted storage one by one, other
    /// storage as bytes.
    pub(super) fn transfer_to_caller(&mut self, source: NodeId) -> Result<(), LowerError> {
        let source = self.graph.resolve(source);
        let destination = self
            .return_pointer
            .ok_or_else(|| LowerError::Malformed("missing hidden return pointer".to_owned()))?;
        let ty = self
            .return_type()
            .ok_or_else(|| LowerError::Malformed("return has no Rust type".to_owned()))?;
        let size = self.layout_of(ty)?.size;
        match self.promotion.place(source) {
            Some((storage, base)) => {
                for lane in self.promotion.lanes(storage).to_vec() {
                    let start = lane.offset.max(base);
                    let end = (lane.offset + u64::from(lane.bytes)).min(base + size);
                    if start >= end {
                        continue;
                    }
                    for (offset, width) in super::memory::chunk_plan(end - start, 8) {
                        let ty = super::integer_type((width * 8) as u8).expect("integer chunk");
                        let part = self.read_storage(storage, start + offset, ty)?;
                        self.store_at(part, destination, start - base + offset);
                    }
                }
            }
            None => {
                let source = self.address(self.storage_source(source))?;
                if source != destination {
                    self.copy_bytes(destination, source, size);
                }
            }
        }
        Ok(())
    }

    pub(super) fn emit_return(&mut self, node: NodeId) -> Result<(), LowerError> {
        let graph = self.graph;
        let mut lanes = Vec::new();
        if let Some(&value) = graph.inputs(node).get(1) {
            if graph.abi.ret == PassMode::Indirect {
                let destination = self.return_pointer.ok_or_else(|| {
                    LowerError::Malformed("missing hidden return pointer".to_owned())
                })?;
                let ty = graph
                    .rust_type(value)
                    .ok_or_else(|| LowerError::Malformed("return has no Rust type".to_owned()))?;
                // A returned merge was written on its incoming edges.
                if !self.promotion.returned(graph.resolve(value)) {
                    self.transfer_to_caller(value)?;
                }
                let _ = ty;
                lanes.push(destination);
            } else {
                self.pass(&mut lanes, value, &graph.abi.ret)?;
            }
        } else if graph.abi.ret == PassMode::Indirect {
            lanes.push(self.return_pointer.ok_or_else(|| {
                LowerError::Malformed("missing hidden return pointer".to_owned())
            })?);
        }
        let expected = self.builder.func.signature.returns.len();
        if lanes.len() != expected {
            return Err(LowerError::Malformed(format!(
                "return carries {} lanes, the signature {expected}",
                lanes.len()
            )));
        }
        self.builder.ins().return_(&lanes);
        Ok(())
    }
}
