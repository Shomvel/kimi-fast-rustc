//! Byte storage for semantic values, shared by interpreted calls.
//!
//! A load produces an owned value snapshot. Initialization is tracked per byte,
//! and enum decoding only visits the active payload. This is not a Rust borrow
//! model: references alone never make an allocation immutable or disjoint.

use super::{InterpretError, Value};
use kfrc_body::{DataObject, FieldLayout, RecordedTypes, TagEncoding, TyId, TypeOracle, TypeShape};
use std::collections::HashMap;

#[derive(Clone)]
struct Allocation {
    base: u64,
    size: u64,
    live: bool,
    mutable: bool,
}

#[derive(Clone, Default)]
pub struct SemanticMemory {
    bytes: HashMap<u64, u8>,
    allocations: Vec<Allocation>,
    next: u64,
    symbols: HashMap<String, u64>,
    definitions: HashMap<String, DataObject>,
}

impl SemanticMemory {
    pub fn new() -> Self {
        Self {
            next: 0x10000,
            ..Self::default()
        }
    }

    /// Register program data before execution. Repeated definitions preserve
    /// current bytes rather than resetting mutations from earlier calls.
    /// Register all callees' definitions up front when they refer to one another.
    pub fn register_data(&mut self, objects: &[DataObject]) -> Result<(), InterpretError> {
        let mut incoming = HashMap::new();
        for object in objects {
            if incoming
                .insert(&object.symbol, object)
                .is_some_and(|existing| existing != object)
            {
                return Err(malformed("conflicting semantic data definitions"));
            }
            if object.thread_local {
                return Err(malformed("TLS is outside the semantic interpreter scope"));
            }
            if let Some(existing) = self.definitions.get(&object.symbol) {
                if existing != object {
                    return Err(malformed("conflicting semantic data definitions"));
                }
            } else if self.symbols.contains_key(&object.symbol) {
                return Err(malformed(
                    "data definition must be registered before its address is used",
                ));
            }
            for relocation in &object.relocations {
                if relocation
                    .offset
                    .checked_add(8)
                    .is_none_or(|end| end > object.bytes.len() as u64)
                {
                    return Err(malformed("data relocation lies outside its object"));
                }
            }
        }
        let mut fresh = Vec::new();
        for object in objects {
            if self.definitions.contains_key(&object.symbol) {
                continue;
            }
            let address =
                self.allocate(object.bytes.len() as u64, object.align, object.writable)?;
            self.initialize_bytes(address, &object.bytes);
            self.symbols.insert(object.symbol.clone(), address);
            self.definitions
                .insert(object.symbol.clone(), object.clone());
            fresh.push(object);
        }
        for object in fresh {
            let base = self.symbols[&object.symbol];
            for relocation in &object.relocations {
                let target = self.symbol_address(&relocation.target)?;
                let bits = target.wrapping_add_signed(relocation.addend);
                self.initialize_bytes(base + relocation.offset, &bits.to_le_bytes());
            }
        }
        Ok(())
    }

    pub(super) fn symbol_address(&mut self, symbol: &str) -> Result<u64, InterpretError> {
        if let Some(address) = self.symbols.get(symbol) {
            return Ok(*address);
        }
        // Opaque external functions have a stable identity but no readable bytes.
        let address = self.allocate(0, 1, false)?;
        self.symbols.insert(symbol.to_owned(), address);
        Ok(address)
    }

    pub fn allocate(
        &mut self,
        size: u64,
        align: u64,
        mutable: bool,
    ) -> Result<u64, InterpretError> {
        if !align.is_power_of_two() {
            return Err(malformed("allocation alignment is not a power of two"));
        }
        let base = self
            .next
            .max(0x10000)
            .checked_add(align - 1)
            .map(|value| value & !(align - 1))
            .ok_or_else(|| malformed("address space exhausted"))?;
        self.next = base
            .checked_add(size.max(1))
            .and_then(|value| value.checked_add(16))
            .ok_or_else(|| malformed("address space exhausted"))?;
        self.allocations.push(Allocation {
            base,
            size,
            live: true,
            mutable,
        });
        Ok(base)
    }

    pub fn read_bytes(&self, address: u64, size: u64) -> Result<Vec<Option<u8>>, InterpretError> {
        self.access(address, size, 1, false)?;
        if size > 16 * 1024 * 1024 {
            return Err(InterpretError::OutOfFuel);
        }
        Ok((0..size)
            .map(|offset| self.bytes.get(&(address + offset)).copied())
            .collect())
    }

    pub fn write_bytes(
        &mut self,
        address: u64,
        bytes: &[Option<u8>],
    ) -> Result<(), InterpretError> {
        self.access(address, bytes.len() as u64, 1, true)?;
        for (offset, byte) in bytes.iter().enumerate() {
            if let Some(byte) = byte {
                self.bytes.insert(address + offset as u64, *byte);
            } else {
                self.bytes.remove(&(address + offset as u64));
            }
        }
        Ok(())
    }

    pub fn load(
        &self,
        types: &RecordedTypes,
        address: u64,
        ty: TyId,
    ) -> Result<Value, InterpretError> {
        let layout = types
            .size_align(ty)
            .ok_or_else(|| malformed("load type has no sized layout"))?;
        self.access(address, layout.size, layout.align, false)?;
        self.decode(types, address, ty)
    }

    pub fn store(
        &mut self,
        types: &RecordedTypes,
        address: u64,
        ty: TyId,
        value: &Value,
    ) -> Result<(), InterpretError> {
        let layout = types
            .size_align(ty)
            .ok_or_else(|| malformed("store type has no sized layout"))?;
        self.access(address, layout.size, layout.align, true)?;
        self.encode(types, address, ty, value)
    }

    pub(super) fn copy_nonoverlapping(
        &mut self,
        source: u64,
        destination: u64,
        size: u64,
        align: u64,
    ) -> Result<(), InterpretError> {
        self.access(source, size, align, false)?;
        self.access(destination, size, align, true)?;
        if size != 0
            && source < destination.saturating_add(size)
            && destination < source.saturating_add(size)
        {
            return Err(ub("overlapping CopyNonOverlapping"));
        }
        let bytes = self.read_bytes(source, size)?;
        self.write_bytes(destination, &bytes)
    }

    pub(super) fn decode_snapshot(
        types: &RecordedTypes,
        address: u64,
        ty: TyId,
        bytes: &HashMap<u64, u8>,
    ) -> Result<Value, InterpretError> {
        Self {
            bytes: bytes.clone(),
            allocations: Vec::new(),
            next: 0,
            ..Self::default()
        }
        .decode(types, address, ty)
    }

    pub(super) fn initialize_bytes(&mut self, address: u64, bytes: &[u8]) {
        for (offset, byte) in bytes.iter().enumerate() {
            self.bytes.insert(address + offset as u64, *byte);
        }
    }

    pub(super) fn lifetime(&mut self, address: u64, live: bool) -> Result<(), InterpretError> {
        let allocation = self
            .allocations
            .iter_mut()
            .find(|allocation| allocation.base == address)
            .ok_or_else(|| malformed("storage lifetime operand is not an allocation root"))?;
        if !allocation.mutable {
            return Err(ub("cannot change static storage lifetime"));
        }
        allocation.live = live;
        let end = address + allocation.size;
        self.bytes.retain(|byte, _| *byte < address || *byte >= end);
        Ok(())
    }

    pub(super) fn set_variant(
        &mut self,
        types: &RecordedTypes,
        address: u64,
        ty: TyId,
        variant: u32,
    ) -> Result<(), InterpretError> {
        let size = types
            .size_align(ty)
            .ok_or_else(|| malformed("enum storage lacks layout"))?;
        self.access(address, size.size, size.align, true)?;
        if types.semantic().field_types(ty, Some(variant)).is_none() {
            return Err(malformed("invalid logical enum variant"));
        }
        let layout = types
            .enums
            .get(&ty)
            .ok_or_else(|| malformed("enum encoding missing"))?;
        let bits = match &layout.encoding {
            TagEncoding::Empty => return Err(ub("constructing an uninhabited enum")),
            TagEncoding::Single {
                variant: single, ..
            } => {
                if variant != *single {
                    return Err(ub("constructing an uninhabited enum variant"));
                }
                return Ok(());
            }
            TagEncoding::Direct { discriminants } => *discriminants
                .get(variant as usize)
                .ok_or_else(|| malformed("enum tag missing"))?,
            TagEncoding::Niche {
                untagged_variant,
                niche_variants,
                niche_start,
            } => {
                if variant == *untagged_variant {
                    return Ok(());
                }
                if !niche_variants.contains(&variant) {
                    return Err(malformed("enum variant outside niche range"));
                }
                niche_start.wrapping_add(u128::from(variant - niche_variants.start()))
            }
        };
        let tag = layout
            .tag
            .ok_or_else(|| malformed("enum tag field missing"))?;
        self.encode(
            types,
            address + tag.offset,
            tag.ty,
            &Value::Scalar(bits as u64),
        )
    }

    fn access(
        &self,
        address: u64,
        size: u64,
        align: u64,
        write: bool,
    ) -> Result<(), InterpretError> {
        if address == 0 || align == 0 || address % align != 0 {
            return Err(ub("null or misaligned memory access"));
        }
        let end = address
            .checked_add(size)
            .ok_or_else(|| ub("memory access overflow"))?;
        let allocation = self
            .allocations
            .iter()
            .find(|allocation| {
                address >= allocation.base && end <= allocation.base + allocation.size
            })
            .ok_or_else(|| ub("memory access outside an allocation"))?;
        if !allocation.live {
            return Err(ub("memory access outside storage lifetime"));
        }
        if write && !allocation.mutable {
            return Err(ub("write to immutable storage"));
        }
        Ok(())
    }

    fn scalar(&self, address: u64, size: u64) -> Result<Value, InterpretError> {
        if size > 8 {
            return Err(malformed("scalar width exceeds 64 bits"));
        }
        let mut bytes = [0; 8];
        for offset in 0..size {
            let Some(byte) = self.bytes.get(&(address + offset)) else {
                return Ok(Value::Poison);
            };
            bytes[offset as usize] = *byte;
        }
        Ok(Value::Scalar(u64::from_le_bytes(bytes)))
    }

    fn decode(
        &self,
        types: &RecordedTypes,
        address: u64,
        ty: TyId,
    ) -> Result<Value, InterpretError> {
        let semantic = types.semantic();
        if semantic.maybe_uninit_inner(ty).is_some() {
            let size = types
                .size_align(ty)
                .ok_or_else(|| malformed("MaybeUninit representation missing"))?
                .size;
            if size > 16 * 1024 * 1024 {
                return Err(InterpretError::OutOfFuel);
            }
            // This helper also decodes an owned ABI snapshot, where no live
            // allocation exists. Public load checked range/liveness already.
            let bytes = (0..size)
                .map(|offset| self.bytes.get(&(address + offset)).copied())
                .collect();
            return Ok(Value::Uninit {
                bytes: std::sync::Arc::new(bytes),
            });
        }

        // Check the enclosing nominal type too: a NonZero newtype's field can
        // be an unrestricted integer. Uninitialized lanes still become poison;
        // a validity constraint must not turn them into an initialized value.
        if let Some(validity) = types.scalar_validity(ty) {
            for (offset, range) in validity.lanes() {
                if let Value::Scalar(bits) = self.scalar(address + offset, u64::from(range.bits()).div_ceil(8))?
                    && !range.contains(u128::from(bits))
                {
                    return Err(ub("scalar representation outside its type's valid range"));
                }
            }
        }

        if semantic.is_enum(ty) {
            let layout = types
                .enums
                .get(&ty)
                .ok_or_else(|| malformed("enum encoding missing"))?;
            let variant = match &layout.encoding {
                TagEncoding::Empty => return Err(ub("loading an uninhabited enum")),
                TagEncoding::Single { variant, .. } => *variant,
                encoding => {
                    let tag = layout.tag.ok_or_else(|| malformed("enum tag missing"))?;
                    let size = types
                        .size_align(tag.ty)
                        .ok_or_else(|| malformed("enum tag layout missing"))?
                        .size;
                    let Value::Scalar(bits) = self.scalar(address + tag.offset, size)? else {
                        return Ok(Value::Poison);
                    };
                    match encoding {
                        TagEncoding::Direct { discriminants } => {
                            let mask = if size == 8 {
                                u64::MAX
                            } else {
                                (1u64 << (size * 8)) - 1
                            };
                            discriminants
                                .iter()
                                .position(|discriminant| *discriminant as u64 & mask == bits)
                                .ok_or_else(|| ub("invalid enum discriminant"))?
                                as u32
                        }
                        TagEncoding::Niche {
                            untagged_variant,
                            niche_variants,
                            niche_start,
                        } => {
                            let mask = if size == 8 {
                                u64::MAX
                            } else {
                                (1u64 << (size * 8)) - 1
                            };
                            let relative = bits.wrapping_sub(*niche_start as u64) & mask;
                            if relative <= u64::from(niche_variants.end() - niche_variants.start())
                            {
                                *niche_variants.start() + relative as u32
                            } else {
                                *untagged_variant
                            }
                        }
                        _ => unreachable!(),
                    }
                }
            };
            let fields = physical_fields(types, ty, Some(variant))?
                .into_iter()
                .map(|field| self.decode(types, address + field.offset, field.ty))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Value::Variant { variant, fields });
        }
        if let TypeShape::Array { element, length } = semantic.shape(ty) {
            if *length == 0 {
                return Ok(Value::array(Value::Poison, 0));
            }
            let stride = types
                .size_align(*element)
                .ok_or_else(|| malformed("array element layout missing"))?
                .size;
            let first = self.decode(types, address, *element)?;
            if stride == 0 {
                return Ok(Value::array(first, *length));
            }
            if *length > 1_000_000 {
                return Err(InterpretError::OutOfFuel);
            }
            let mut overrides = HashMap::new();
            for index in 1..*length {
                let value = self.decode(types, address + index * stride, *element)?;
                if value != first {
                    overrides.insert(index, value);
                }
            }
            return Ok(Value::Array {
                element: std::sync::Arc::new(first),
                length: *length,
                overrides: std::sync::Arc::new(overrides),
            });
        }
        if semantic.is_product(ty) {
            let fields = physical_fields(types, ty, None)?
                .into_iter()
                .map(|field| self.decode(types, address + field.offset, field.ty))
                .collect::<Result<Vec<_>, _>>()?;
            return if matches!(
                semantic.shape(ty),
                TypeShape::RawPointer { .. } | TypeShape::Reference { .. }
            ) {
                let [data, metadata]: [Value; 2] = fields
                    .try_into()
                    .map_err(|_| malformed("wide pointer needs two fields"))?;
                Ok(Value::WidePointer {
                    data: Box::new(data),
                    metadata: Box::new(metadata),
                })
            } else {
                Ok(Value::Aggregate(fields))
            };
        }
        let size = types
            .size_align(ty)
            .ok_or_else(|| malformed("scalar layout missing"))?
            .size;
        let value = self.scalar(address, size)?;
        if let Value::Scalar(bits) = value {
            match semantic.shape(ty) {
                TypeShape::Bool if bits > 1 => return Err(ub("invalid bool in memory")),
                TypeShape::Char if char::from_u32(bits as u32).is_none() => {
                    return Err(ub("invalid char in memory"));
                }
                _ => {}
            }
        }
        Ok(value)
    }

    fn encode(
        &mut self,
        types: &RecordedTypes,
        address: u64,
        ty: TyId,
        value: &Value,
    ) -> Result<(), InterpretError> {
        let size = types
            .size_align(ty)
            .ok_or_else(|| malformed("value layout missing"))?
            .size;
        if matches!(value, Value::Poison) {
            for offset in 0..size {
                self.bytes.remove(&(address + offset));
            }
            return Ok(());
        }
        let semantic = types.semantic();
        if semantic.maybe_uninit_inner(ty).is_some() {
            let Value::Uninit { bytes } = value else {
                return Err(malformed(
                    "MaybeUninit store requires an opaque representation",
                ));
            };
            if bytes.len() as u64 != size {
                return Err(malformed("MaybeUninit representation size mismatch"));
            }
            for (offset, byte) in bytes.iter().enumerate() {
                let address = address + offset as u64;
                match byte {
                    Some(byte) => {
                        self.bytes.insert(address, *byte);
                    }
                    None => {
                        self.bytes.remove(&address);
                    }
                }
            }
            return Ok(());
        }

        match value {
            Value::Scalar(bits) if !semantic.is_product(ty) && !semantic.is_enum(ty) => {
                if size > 8 {
                    return Err(malformed("scalar store wider than 64 bits"));
                }
                for offset in 0..size {
                    self.bytes
                        .insert(address + offset, bits.to_le_bytes()[offset as usize]);
                }
            }
            Value::Array { length: count, .. } => {
                let TypeShape::Array { element, length } = semantic.shape(ty) else {
                    return Err(malformed("compact array store to non-array type"));
                };
                if count != length {
                    return Err(malformed("compact array store length mismatch"));
                }
                let stride = types
                    .size_align(*element)
                    .ok_or_else(|| malformed("array element layout missing"))?
                    .size;
                if stride == 0 || *length == 0 {
                    return Ok(());
                }
                if *length > 1_000_000 {
                    return Err(InterpretError::OutOfFuel);
                }
                for index in 0..*length {
                    self.encode(
                        types,
                        address + index * stride,
                        *element,
                        value.array_element(index).expect("array index checked"),
                    )?;
                }
            }
            Value::Aggregate(fields) => {
                let layouts = physical_fields(types, ty, None)?;
                if fields.len() != layouts.len() {
                    return Err(malformed("aggregate store field mismatch"));
                }
                for (field, layout) in fields.iter().zip(layouts) {
                    self.encode(types, address + layout.offset, layout.ty, field)?;
                }
            }
            Value::WidePointer { data, metadata } => {
                let fields = physical_fields(types, ty, None)?;
                if fields.len() != 2 {
                    return Err(malformed("wide pointer store needs two fields"));
                }
                for (value, field) in [data.as_ref(), metadata.as_ref()].into_iter().zip(fields) {
                    self.encode(types, address + field.offset, field.ty, value)?;
                }
            }
            Value::Variant { variant, fields } if semantic.is_enum(ty) => {
                let layouts = physical_fields(types, ty, Some(*variant))?;
                if fields.len() != layouts.len() {
                    return Err(malformed("variant store field mismatch"));
                }
                // Payload first: niche-bearing fields must survive selection of the
                // untagged variant; tagged variants only overwrite their tag bytes.
                for (value, field) in fields.iter().zip(layouts) {
                    self.encode(types, address + field.offset, field.ty, value)?;
                }
                self.set_variant(types, address, ty, *variant)?;
            }
            _ => return Err(malformed("store value does not match destination type")),
        }
        Ok(())
    }
}

pub(super) fn physical_fields(
    types: &RecordedTypes,
    ty: TyId,
    variant: Option<u32>,
) -> Result<Vec<FieldLayout>, InterpretError> {
    if let TypeShape::Unit = types.shape(ty) {
        return Ok(Vec::new());
    }
    if let TypeShape::Array { element, length } = types.shape(ty) {
        if *length > 1_000_000 {
            return Err(InterpretError::OutOfFuel);
        }
        let stride = types
            .size_align(*element)
            .ok_or_else(|| malformed("array element layout missing"))?
            .size;
        return Ok((0..*length)
            .map(|index| FieldLayout {
                ty: *element,
                offset: index * stride,
            })
            .collect());
    }
    let layout = types
        .aggregates
        .get(&ty)
        .ok_or_else(|| malformed("aggregate field layout missing"))?;
    match variant {
        Some(variant) => layout
            .variants
            .get(variant as usize)
            .cloned()
            .ok_or_else(|| malformed("variant field layout missing")),
        None => Ok(layout.fields.clone()),
    }
}

fn malformed(message: impl Into<String>) -> InterpretError {
    InterpretError::Malformed(message.into())
}
fn ub(message: impl Into<String>) -> InterpretError {
    InterpretError::UndefinedBehaviour(message.into())
}
