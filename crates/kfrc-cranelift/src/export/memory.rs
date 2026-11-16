//! Frame storage, loads and stores, and small constant-size memory routines.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{self, InstBuilder, MemFlagsData, StackSlotData, StackSlotKind, Value};
use cranelift_codegen::ir::{Type, types};
use kfrc_body::{RuntimeComparison, SizeAlign, TyId, TypeOracle};
use kfrc_son::{NodeId, Opcode};

use super::{Exporter, LowerError, Scalar, integer_type};

/// Cranelift aligns the frame to sixteen bytes; stronger slot alignment is
/// reached by rounding the address inside an enlarged slot.
const FRAME_ALIGNMENT: u64 = 16;

/// The widest chunk a small copy moves at once: one SSE2 register.
const WIDEST_COPY_CHUNK: u64 = 16;
/// The widest chunk a small comparison combines: one general register.
const WIDEST_EQUALITY_CHUNK: u64 = 8;

/// Rust memory is addressable wherever the graph accesses it; alignment is a
/// property of the place, which may be packed.
pub(super) fn memory_flags() -> MemFlagsData {
    MemFlagsData::new().with_notrap()
}

/// Chunks `(offset, width)` covering exactly `[0, size)`. Widths are powers of
/// two up to `widest`. A tail that would need three or more chunks is covered
/// by two chunks of one width, the second ending at `size` and overlapping
/// the first. Every chunk lies inside the range.
pub(super) fn chunk_plan(size: u64, widest: u64) -> Vec<(u64, u64)> {
    let mut chunks = Vec::new();
    let mut offset = 0;
    while offset < size {
        let remaining = size - offset;
        let width = widest.min(1 << remaining.ilog2());
        let rest = remaining - width;
        if rest != 0 && rest < width && !rest.is_power_of_two() {
            chunks.push((offset, width));
            chunks.push((size - width, width));
            return chunks;
        }
        chunks.push((offset, width));
        offset += width;
    }
    chunks
}

impl Exporter<'_, '_> {
    /// A fresh frame allocation with `layout`, and its address.
    pub(super) fn new_stack_storage(&mut self, layout: SizeAlign) -> Result<Value, LowerError> {
        let (slot, align) = self.create_stack_slot(layout)?;
        Ok(self.stack_address(slot, align))
    }

    pub(super) fn new_stack_storage_of(&mut self, ty: TyId) -> Result<Value, LowerError> {
        let layout = self.layout_of(ty)?;
        self.new_stack_storage(layout)
    }

    pub(super) fn layout_of(&self, ty: TyId) -> Result<SizeAlign, LowerError> {
        self.graph.oracle.size_align(ty).ok_or_else(|| {
            LowerError::Unsupported(format!(
                "unsized value `{}`",
                self.graph.oracle.describe(ty)
            ))
        })
    }

    fn create_stack_slot(&mut self, layout: SizeAlign) -> Result<(ir::StackSlot, u64), LowerError> {
        let align = layout.align.max(1);
        if !align.is_power_of_two() {
            return Err(LowerError::Unsupported(format!(
                "stack alignment {align} is not a power of two"
            )));
        }
        let padding = if align > FRAME_ALIGNMENT { align } else { 0 };
        let size = layout
            .size
            .checked_add(padding)
            .and_then(|size| u32::try_from(size).ok())
            .ok_or_else(|| LowerError::Unsupported("frame allocation beyond 4 GiB".into()))?;
        let slot = self.builder.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            size,
            align.min(FRAME_ALIGNMENT).ilog2() as u8,
        ));
        Ok((slot, align))
    }

    fn stack_address(&mut self, slot: ir::StackSlot, align: u64) -> Value {
        let address = self.builder.ins().stack_addr(types::I64, slot, 0);
        if align <= FRAME_ALIGNMENT {
            return address;
        }
        let rounded = self.builder.ins().iadd_imm_u(address, (align - 1) as i64);
        self.builder.ins().band_imm_s(rounded, -(align as i64))
    }

    /// The address of a graph stack slot. Every use recomputes it from the
    /// frame; Cranelift folds it into the addressing mode of the access.
    pub(super) fn stack_slot_address(&mut self, index: usize) -> Result<Value, LowerError> {
        let layout =
            *self.graph.stack_slots.get(index).ok_or_else(|| {
                LowerError::Malformed(format!("stack slot {index} is not declared"))
            })?;
        let slot = match self.stack_slots[index] {
            Some(slot) => slot,
            None => {
                let (slot, _) = self.create_stack_slot(layout)?;
                self.stack_slots[index] = Some(slot);
                slot
            }
        };
        Ok(self.stack_address(slot, layout.align.max(1)))
    }

    pub(super) fn offset_address(&mut self, address: Value, offset: u64) -> Value {
        if offset == 0 {
            address
        } else {
            self.builder.ins().iadd_imm_u(address, offset as i64)
        }
    }

    pub(super) fn load_at(&mut self, ty: Type, address: Value, offset: u64) -> Value {
        match i32::try_from(offset) {
            Ok(displacement) => self
                .builder
                .ins()
                .load(ty, memory_flags(), address, displacement),
            Err(_) => {
                let address = self.offset_address(address, offset);
                self.builder.ins().load(ty, memory_flags(), address, 0)
            }
        }
    }

    pub(super) fn store_at(&mut self, value: Value, address: Value, offset: u64) {
        match i32::try_from(offset) {
            Ok(displacement) => {
                self.builder
                    .ins()
                    .store(memory_flags(), value, address, displacement);
            }
            Err(_) => {
                let address = self.offset_address(address, offset);
                self.builder.ins().store(memory_flags(), value, address, 0);
            }
        }
    }

    /// Little-endian integer of `count` (1..=8) object bytes, zero-extended
    /// to 64 bits. Only the named bytes are read.
    pub(super) fn load_raw_bytes(&mut self, address: Value, offset: u64, count: u8) -> Value {
        let mut result = None;
        for (chunk_offset, width) in chunk_plan(u64::from(count), 8) {
            let ty = integer_type((width * 8) as u8).expect("power-of-two chunk");
            let part = self.load_at(ty, address, offset + chunk_offset);
            let part = self.coerce(part, Scalar::ADDRESS, 64);
            let part = if chunk_offset == 0 {
                part
            } else {
                self.builder
                    .ins()
                    .ishl_imm_u(part, (chunk_offset * 8) as i64)
            };
            result = Some(match result {
                None => part,
                Some(previous) => self.builder.ins().bor(previous, part),
            });
        }
        result.unwrap_or_else(|| self.builder.ins().iconst(types::I64, 0))
    }

    /// Writes the low `count` (1..=8) bytes of a 64-bit `value`.
    pub(super) fn store_raw_bytes(&mut self, address: Value, offset: u64, count: u8, value: Value) {
        for (chunk_offset, width) in chunk_plan(u64::from(count), 8) {
            let part = if chunk_offset == 0 {
                value
            } else {
                self.builder
                    .ins()
                    .ushr_imm_u(value, (chunk_offset * 8) as i64)
            };
            let part = self.coerce(part, Scalar::ADDRESS, (width * 8) as u8);
            self.store_at(part, address, offset + chunk_offset);
        }
    }

    /// The integer of `ty` at `offset` bytes from the address node `place`:
    /// a lane variable of promoted storage, or a memory load.
    pub(super) fn read_place(
        &mut self,
        place: NodeId,
        offset: u64,
        ty: Type,
    ) -> Result<Value, LowerError> {
        let place = self.graph.resolve(place);
        match self.promotion.place(place) {
            Some((storage, base)) => self.read_storage(storage, base + offset, ty),
            None => {
                let address = self.address(place)?;
                Ok(self.load_at(ty, address, offset))
            }
        }
    }

    /// Writes an integer at `offset` bytes from the address node `place`.
    pub(super) fn write_place(
        &mut self,
        place: NodeId,
        offset: u64,
        value: Value,
    ) -> Result<(), LowerError> {
        let place = self.graph.resolve(place);
        match self.promotion.place(place) {
            Some((storage, base)) => {
                self.write_storage(storage, base + offset, value)?;
            }
            None => {
                let address = self.address(place)?;
                self.store_at(value, address, offset);
            }
        }
        Ok(())
    }

    /// The intersections of a byte range with its disjoint storage lanes.
    /// They must cover the entire access, without gaps or reading outside it.
    fn storage_parts(
        &self,
        storage: usize,
        offset: u64,
        ty: Type,
    ) -> Result<Vec<(super::promote::Lane, u64, u8)>, LowerError> {
        let end = offset
            .checked_add(u64::from(ty.bytes()))
            .ok_or_else(|| LowerError::Malformed("storage access range overflow".into()))?;
        let mut parts = Vec::new();
        let mut cursor = offset;
        for &lane in self.promotion.lanes(storage) {
            let start = lane.offset.max(offset);
            let stop = (lane.offset + u64::from(lane.bytes)).min(end);
            if start < stop {
                if start != cursor {
                    break;
                }
                parts.push((lane, start, (stop - start) as u8));
                cursor = stop;
            }
        }
        if cursor != end {
            return Err(LowerError::Malformed(format!(
                "promoted storage does not cover {} bytes at offset {offset}",
                ty.bytes()
            )));
        }
        Ok(parts)
    }

    /// Reassemble exactly the requested bytes, in x86's little-endian order.
    /// This is raw transport: reading an integer lane grants no Rust validity
    /// or pointer dereferenceability to the bits it contains.
    pub(super) fn read_storage(
        &mut self,
        storage: usize,
        offset: u64,
        ty: Type,
    ) -> Result<Value, LowerError> {
        let mut value = None;
        for (lane, start, count) in self.storage_parts(storage, offset, ty)? {
            let mut part = self.builder.use_var(lane.variable);
            if start != lane.offset {
                part = self
                    .builder
                    .ins()
                    .ushr_imm_u(part, ((start - lane.offset) * 8) as i64);
            }
            part = self.coerce(part, Scalar::ADDRESS, ty.bits() as u8);
            if u32::from(count) * 8 < ty.bits() {
                part = self
                    .builder
                    .ins()
                    .band_imm_u(part, ((1u64 << (count * 8)) - 1) as i64);
            }
            if start != offset {
                part = self
                    .builder
                    .ins()
                    .ishl_imm_u(part, ((start - offset) * 8) as i64);
            }
            value = Some(match value {
                None => part,
                Some(previous) => self.builder.ins().bor(previous, part),
            });
        }
        value.ok_or_else(|| LowerError::Malformed("empty storage access".into()))
    }

    /// Replace only the named bytes. A narrow write preserves every other
    /// byte, including bytes initialized by an earlier wider access or copy.
    pub(super) fn write_storage(
        &mut self,
        storage: usize,
        offset: u64,
        value: Value,
    ) -> Result<(), LowerError> {
        let ty = self.builder.func.dfg.value_type(value);
        for (lane, start, count) in self.storage_parts(storage, offset, ty)? {
            let mut part = value;
            if start != offset {
                part = self
                    .builder
                    .ins()
                    .ushr_imm_u(part, ((start - offset) * 8) as i64);
            }
            part = self.coerce(part, Scalar::ADDRESS, lane.ty().bits() as u8);
            if count != lane.bytes {
                let shift = (start - lane.offset) * 8;
                let mask = (1u64 << (count * 8)) - 1;
                part = self.builder.ins().band_imm_u(part, mask as i64);
                if shift != 0 {
                    part = self.builder.ins().ishl_imm_u(part, shift as i64);
                }
                let previous = self.builder.use_var(lane.variable);
                let preserved = self
                    .builder
                    .ins()
                    .band_imm_u(previous, !(mask << shift) as i64);
                part = self.builder.ins().bor(preserved, part);
            }
            self.builder.def_var(lane.variable, part);
        }
        Ok(())
    }

    pub(super) fn emit_load(&mut self, node: NodeId) -> Result<(), LowerError> {
        // A load whose value nobody reads has no observable behaviour.
        let Some(result) = self.projection(node, 2) else {
            return Ok(());
        };
        let Some(ty) = self.scalar_of_node(result)?.ty() else {
            return Ok(());
        };
        let value = self.read_place(self.graph.input(node, 1), 0, ty)?;
        self.define(result, value);
        Ok(())
    }

    pub(super) fn emit_store(&mut self, node: NodeId) -> Result<(), LowerError> {
        let stored = self.graph.input(node, 2);
        if self.scalar_of_node(stored)?.bits == 0 {
            return Ok(());
        }
        let value = self.value(stored)?;
        self.write_place(self.graph.input(node, 1), 0, value)
    }

    /// Copies `size` bytes between storage that is either disjoint or the
    /// same: each chunk is read and written before the next.
    pub(super) fn copy_bytes(&mut self, destination: Value, source: Value, size: u64) {
        if size > self.options.max_unrolled_memory_bytes {
            self.call_runtime_copy("memmove", destination, source, size);
            return;
        }
        for (offset, width) in chunk_plan(size, WIDEST_COPY_CHUNK) {
            let ty = chunk_type(width);
            let value = self.load_at(ty, source, offset);
            self.store_at(value, destination, offset);
        }
    }

    fn call_runtime_copy(&mut self, name: &str, destination: Value, source: Value, size: u64) {
        let mut signature = ir::Signature::new(cranelift_codegen::isa::CallConv::SystemV);
        for _ in 0..3 {
            signature.params.push(ir::AbiParam::new(types::I64));
        }
        signature.returns.push(ir::AbiParam::new(types::I64));
        let signature = self.signature_reference(signature);
        let function = self.function_reference(name, signature);
        let size = self.builder.ins().iconst(types::I64, size as i64);
        self.builder
            .ins()
            .call(function, &[destination, source, size]);
    }

    /// A constant-size copy snapshots the entire source range before writing.
    /// Each side can have different lanes, or one side can be real memory.
    /// This also handles overlapping ranges within one promoted storage.
    pub(super) fn emit_lane_copy(&mut self, node: NodeId) -> Result<bool, LowerError> {
        let Some((destination, source, bytes)) = super::promote::constant_copy(self.graph, node)
        else {
            return Ok(false);
        };
        if self.promotion.place(destination).is_none() && self.promotion.place(source).is_none() {
            return Ok(false);
        }
        if destination == source {
            return Ok(true);
        }
        let chunks = chunk_plan(bytes, 8);
        let mut values = Vec::with_capacity(chunks.len());
        for &(offset, width) in &chunks {
            values.push(self.read_place(source, offset, chunk_type(width))?);
        }
        for ((offset, _), value) in chunks.into_iter().zip(values) {
            self.write_place(destination, offset, value)?;
        }
        Ok(true)
    }

    /// `memmove`/`memcpy` of a small constant size becomes loads and stores.
    /// Every chunk is read before any is written, which keeps an overlapping
    /// `memmove` exact.
    pub(super) fn emit_small_copy(&mut self, node: NodeId) -> Result<bool, LowerError> {
        const MAX_BYTES: u64 = 64;
        let graph = self.graph;
        if graph.op(node) != Opcode::Call || graph.inputs(node).len() != 4 {
            return Ok(false);
        }
        let symbol = &graph.callees[graph.payload(node) as usize].symbol;
        let count = graph.resolve(graph.input(node, 3));
        if !matches!(symbol.as_str(), "memmove" | "memcpy") || graph.op(count) != Opcode::Const {
            return Ok(false);
        }
        let size = graph.constant_value(count);
        if size > u128::from(MAX_BYTES) {
            return Ok(false);
        }
        let destination = self.address(graph.input(node, 1))?;
        let source = self.address(graph.input(node, 2))?;
        let chunks = chunk_plan(size as u64, WIDEST_COPY_CHUNK);
        let values: Vec<Value> = chunks
            .iter()
            .map(|&(offset, width)| self.load_at(chunk_type(width), source, offset))
            .collect();
        for (&(offset, _), value) in chunks.iter().zip(values) {
            self.store_at(value, destination, offset);
        }
        if let Some(result) = self.projection(node, 2) {
            self.define(result, destination);
        }
        Ok(true)
    }

    /// An authenticated byte equality of a small constant size becomes loads,
    /// XORs and one comparison. A same-named user call carries no read
    /// contract and stays a call.
    pub(super) fn emit_small_equality(&mut self, node: NodeId) -> Result<bool, LowerError> {
        const MAX_BYTES: u64 = 32;
        let graph = self.graph;
        if graph.op(node) != Opcode::Call || graph.inputs(node).len() != 4 {
            return Ok(false);
        }
        let count = graph.resolve(graph.input(node, 3));
        if graph.runtime_comparison(graph.payload(node)) != Some(RuntimeComparison::Bcmp)
            || graph.op(count) != Opcode::Const
            || graph.constant_value(count) > u128::from(MAX_BYTES)
        {
            return Ok(false);
        }
        let size = graph.constant_value(count) as u64;
        let left = self.address(graph.input(node, 1))?;
        let right = self.address(graph.input(node, 2))?;
        let mut difference = None;
        for (offset, width) in chunk_plan(size, WIDEST_EQUALITY_CHUNK) {
            let ty = chunk_type(width);
            let a = self.load_at(ty, left, offset);
            let b = self.load_at(ty, right, offset);
            let chunk = self.builder.ins().bxor(a, b);
            let chunk = self.coerce(chunk, Scalar::ADDRESS, 64);
            difference = Some(match difference {
                None => chunk,
                Some(previous) => self.builder.ins().bor(previous, chunk),
            });
        }
        if let Some(result) = self.projection(node, 2) {
            let unequal = match difference {
                Some(difference) => self
                    .builder
                    .ins()
                    .icmp_imm_u(IntCC::NotEqual, difference, 0),
                None => self.builder.ins().iconst(types::I8, 0),
            };
            let scalar = self.scalar_of_node(result)?;
            let value = self.coerce(
                unequal,
                Scalar {
                    bits: 8,
                    signed: false,
                },
                scalar.bits,
            );
            self.define(result, value);
        }
        Ok(true)
    }
}

fn chunk_type(width: u64) -> Type {
    match width {
        16 => types::I8X16,
        other => integer_type((other * 8) as u8).expect("power-of-two chunk"),
    }
}

#[cfg(test)]
mod tests {
    use super::chunk_plan;

    #[test]
    fn plans_cover_exactly_the_range_with_few_chunks() {
        for widest in [8, 16] {
            for size in 0..=128u64 {
                let chunks = chunk_plan(size, widest);
                let mut covered = vec![false; size as usize];
                for &(offset, width) in &chunks {
                    assert!(width.is_power_of_two() && width <= widest);
                    assert!(
                        offset + width <= size,
                        "size={size} chunk=({offset},{width})"
                    );
                    covered[offset as usize..(offset + width) as usize].fill(true);
                }
                assert!(covered.iter().all(|byte| *byte), "size={size}");
                let full = size / widest;
                assert!(chunks.len() as u64 <= full + 2, "size={size}: {chunks:?}");
            }
        }
        assert_eq!(chunk_plan(7, 8), vec![(0, 4), (3, 4)]);
        assert_eq!(chunk_plan(15, 8), vec![(0, 8), (7, 8)]);
        assert_eq!(chunk_plan(31, 16), vec![(0, 16), (15, 16)]);
        assert_eq!(chunk_plan(24, 16), vec![(0, 16), (16, 8)]);
        assert_eq!(chunk_plan(3, 8), vec![(0, 2), (2, 1)]);
        assert_eq!(chunk_plan(23, 16), vec![(0, 16), (7, 16)]);
    }
}
