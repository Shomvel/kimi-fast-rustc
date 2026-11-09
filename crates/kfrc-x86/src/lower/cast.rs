//! Exact integer-lane coercions selected by rustc's per-call CastTarget.
use super::*;
use kfrc_body::{CastAbi, CastExtension, CastLane, FunctionAbi};

impl Lowering<'_> {
    pub(super) fn cast_lanes(cast: CastAbi) -> Result<Vec<CastLane>, LowerError> {
        let lanes = cast.lanes.into_iter().flatten().collect::<Vec<_>>();
        if lanes.iter().any(|lane| !(1..=8).contains(&lane.bytes)) {
            return Err(LowerError::Unsupported(
                "Cast ABI contains a non-GPR lane".to_owned(),
            ));
        }
        if cast.extension != CastExtension::None && lanes.len() != 1 {
            return Err(LowerError::Unsupported(
                "Cast ABI extension on multiple lanes".to_owned(),
            ));
        }
        Ok(lanes)
    }

    /// Little-endian integer loads assembled from valid object bytes. Unlike
    /// a wide load followed by a mask, this never reads past a partial tail.
    pub(super) fn load_raw_bytes(
        &mut self,
        address: VirtualValue,
        offset: u64,
        count: u8,
    ) -> VirtualValue {
        let result = self.temporary();
        self.push(VirtualInstr::Mov {
            dst: result,
            src: VirtualValue::Imm(0),
        });
        let mut copied = 0;
        while copied < count {
            let bytes = match count - copied {
                8.. => 8,
                4..=7 => 4,
                2..=3 => 2,
                _ => 1,
            };
            let part = self.temporary();
            let source = self.offset_address(address, offset + u64::from(copied));
            self.instructions.push(LowerInstr::Load {
                dst: part,
                address: source,
                bytes,
            });
            let part = if copied == 0 {
                VirtualValue::Reg(part)
            } else {
                let shifted = self.temporary();
                self.push(VirtualInstr::Shl {
                    dst: shifted,
                    lhs: VirtualValue::Reg(part),
                    rhs: VirtualValue::Imm(i64::from(copied) * 8),
                });
                VirtualValue::Reg(shifted)
            };
            self.push(VirtualInstr::Or {
                dst: result,
                lhs: VirtualValue::Reg(result),
                rhs: part,
            });
            copied += bytes;
        }
        VirtualValue::Reg(result)
    }

    pub(super) fn store_raw_bytes(
        &mut self,
        address: VirtualValue,
        offset: u64,
        count: u8,
        value: VirtualValue,
    ) {
        let mut copied = 0;
        while copied < count {
            let bytes = match count - copied {
                8.. => 8,
                4..=7 => 4,
                2..=3 => 2,
                _ => 1,
            };
            let source = if copied == 0 {
                value
            } else {
                let shifted = self.temporary();
                self.push(VirtualInstr::LShr {
                    dst: shifted,
                    lhs: value,
                    rhs: VirtualValue::Imm(i64::from(copied) * 8),
                });
                VirtualValue::Reg(shifted)
            };
            let destination = self.offset_address(address, offset + u64::from(copied));
            self.instructions.push(LowerInstr::Store {
                address: destination,
                src: source,
                bytes,
            });
            copied += bytes;
        }
    }

    pub(super) fn value_address(&mut self, value: NodeId) -> Result<VirtualValue, LowerError> {
        let ty = self
            .graph
            .rust_type(value)
            .ok_or_else(|| LowerError::Malformed("Cast operand lacks Rust type".to_owned()))?;
        let value = self.value(value)?;
        if let Ok(kind) = self.scalar_kind(ty) {
            let address = self.temporary();
            self.allocate_slot(address, ty)?;
            self.store_raw_bytes(VirtualValue::Reg(address), 0, (kind.bits / 8) as u8, value);
            Ok(VirtualValue::Reg(address))
        } else {
            Ok(value)
        }
    }

    pub(super) fn load_cast(
        &mut self,
        value: NodeId,
        cast: CastAbi,
    ) -> Result<Vec<VirtualValue>, LowerError> {
        let address = self.value_address(value)?;
        Self::cast_lanes(cast)?
            .into_iter()
            .map(|lane| {
                let count = u64::from(lane.bytes).min(cast.size.saturating_sub(lane.offset)) as u8;
                let value = self.load_raw_bytes(address, lane.offset, count);
                if cast.extension == CastExtension::None {
                    return Ok(value);
                }
                let extended = self.temporary();
                self.canonicalize(
                    extended,
                    value,
                    ScalarKind {
                        bits: u32::from(lane.bytes) * 8,
                        signed: cast.extension == CastExtension::Sign,
                    },
                );
                Ok(VirtualValue::Reg(extended))
            })
            .collect()
    }

    pub(super) fn receive_cast(
        &mut self,
        result: NodeId,
        cast: CastAbi,
        values: &[VirtualValue],
    ) -> Result<(), LowerError> {
        let ty = self
            .graph
            .rust_type(result)
            .ok_or_else(|| LowerError::Malformed("Cast result lacks Rust type".to_owned()))?;
        let scalar = self.scalar_kind(ty).ok();
        let address = if scalar.is_some() {
            self.temporary()
        } else {
            register(result)
        };
        self.allocate_slot(address, ty)?;
        let lanes = Self::cast_lanes(cast)?;
        if lanes.len() != values.len() {
            return Err(LowerError::Malformed(
                "Cast lane/value count mismatch".to_owned(),
            ));
        }
        for (lane, value) in lanes.into_iter().zip(values) {
            let count = u64::from(lane.bytes).min(cast.size.saturating_sub(lane.offset)) as u8;
            self.store_raw_bytes(VirtualValue::Reg(address), lane.offset, count, *value);
        }
        if let Some(kind) = scalar {
            let value = self.load_raw_bytes(VirtualValue::Reg(address), 0, (kind.bits / 8) as u8);
            self.canonicalize(register(result), value, kind);
        }
        self.emitted.insert(result);
        Ok(())
    }

    pub(super) fn emit_cast_call(
        &mut self,
        node: NodeId,
        abi: FunctionAbi,
        target: CallDestination,
        first_arg: usize,
    ) -> Result<(), LowerError> {
        let graph = self.graph;
        let plan = super::vector::argument_plan(&abi)?;
        let mut vector_args = Vec::new();
        let mut args = Vec::new();
        let result = self.schedule.projections.get(&(node, 2)).copied();
        if abi.ret == PassMode::Indirect {
            let ty = match graph.value_type(node) {
                ValueType::Tuple(elements) => match elements.get(2) {
                    Some(ValueType::Rust(ty) | ValueType::LayoutValue(ty)) => *ty,
                    _ => {
                        return Err(LowerError::Malformed(
                            "sret call lacks result type".to_owned(),
                        ));
                    }
                },
                _ => {
                    return Err(LowerError::Malformed(
                        "sret call lacks tuple result".to_owned(),
                    ));
                }
            };
            let address = result.map(register).unwrap_or_else(|| self.temporary());
            self.allocate_slot(address, ty)?;
            args.push((0, VirtualValue::Reg(address)));
            if let Some(result) = result {
                self.emitted.insert(result);
            }
        }
        for (index, ((input, mode), locations)) in graph.inputs(node)[first_arg..]
            .iter()
            .zip(&abi.args)
            .zip(plan.integers)
            .enumerate()
        {
            let ty = graph
                .rust_type(*input)
                .ok_or_else(|| LowerError::Malformed("call operand lacks Rust type".to_owned()))?;
            if let Some(location) = plan.vectors[index] {
                let address = self.value(*input)?;
                match location {
                    super::vector::VectorLocation::Register(reg) => {
                        vector_args.push((reg, address))
                    }
                    super::vector::VectorLocation::Stack(slot) => {
                        for lane in 0..2 {
                            let value = self.load_raw_bytes(address, lane as u64 * 8, 8);
                            args.push((slot + lane, value));
                        }
                    }
                }
                continue;
            }
            let values = match mode {
                PassMode::Ignore => Vec::new(),
                PassMode::Direct if self.scalar_kind(ty).is_ok() => vec![self.value(*input)?],
                PassMode::Direct => {
                    let address = self.value(*input)?;
                    vec![self.load_raw_bytes(address, 0, self.direct_bits(ty)?)]
                }
                PassMode::Indirect => vec![self.value(*input)?],
                PassMode::Pair => {
                    let address = self.value(*input)?;
                    let fields = *graph.oracle.abi_pairs.get(&ty).ok_or_else(|| {
                        LowerError::Unsupported("missing pair call layout".to_owned())
                    })?;
                    fields
                        .into_iter()
                        .map(|field| self.load_field(address, field.offset, field.ty))
                        .collect::<Result<Vec<_>, _>>()?
                }
                PassMode::Cast(cast) => {
                    let mut values = if cast.pad_i32 {
                        vec![VirtualValue::Imm(0)]
                    } else {
                        Vec::new()
                    };
                    values.extend(self.load_cast(*input, *cast)?);
                    values
                }
                other => {
                    return Err(LowerError::Unsupported(format!(
                        "call argument passed as {other:?}"
                    )));
                }
            };
            if locations.len() != values.len() {
                return Err(LowerError::Malformed(
                    "argument plan length mismatch".to_owned(),
                ));
            }
            args.extend(locations.into_iter().zip(values));
        }
        let count = match abi.ret {
            PassMode::Ignore | PassMode::Indirect | PassMode::Vector { .. } => 0,
            PassMode::Pair => 2,
            PassMode::Cast(cast) => Self::cast_lanes(cast)?.len(),
            _ => 1,
        };
        let values = (0..count).map(|_| self.temporary()).collect::<Vec<_>>();
        let vector_result = if matches!(abi.ret, PassMode::Vector { .. }) {
            if let Some(result) = result {
                let ty = graph
                    .rust_type(result)
                    .ok_or_else(|| LowerError::Malformed("vector result lacks type".into()))?;
                self.allocate_slot(register(result), ty)?;
                Some(VirtualValue::Reg(register(result)))
            } else {
                None
            }
        } else {
            None
        };
        self.instructions.push(LowerInstr::AbiCall {
            results: values.clone(),
            target,
            args,
            vector_args,
            vector_result,
        });
        let Some(result) =
            result.filter(|_| !matches!(abi.ret, PassMode::Ignore | PassMode::Indirect))
        else {
            self.emitted.insert(node);
            return Ok(());
        };
        let ty = graph
            .rust_type(result)
            .ok_or_else(|| LowerError::Malformed("call result lacks Rust type".to_owned()))?;
        match abi.ret {
            PassMode::Vector { bytes: 16 } => {}
            PassMode::Cast(cast) => self.receive_cast(
                result,
                cast,
                &values
                    .iter()
                    .copied()
                    .map(VirtualValue::Reg)
                    .collect::<Vec<_>>(),
            )?,
            PassMode::Pair => {
                let fields = *graph.oracle.abi_pairs.get(&ty).ok_or_else(|| {
                    LowerError::Unsupported("missing pair result layout".to_owned())
                })?;
                self.allocate_slot(register(result), ty)?;
                for (field, value) in fields.into_iter().zip(values) {
                    self.store_field(
                        VirtualValue::Reg(register(result)),
                        field.offset,
                        field.ty,
                        VirtualValue::Reg(value),
                    )?;
                }
            }
            PassMode::Direct if self.scalar_kind(ty).is_ok() => self.canonicalize(
                register(result),
                VirtualValue::Reg(values[0]),
                self.scalar_kind(ty)?,
            ),
            PassMode::Direct => {
                self.allocate_slot(register(result), ty)?;
                self.store_raw_bytes(
                    VirtualValue::Reg(register(result)),
                    0,
                    self.direct_bits(ty)?,
                    VirtualValue::Reg(values[0]),
                );
            }
            other => {
                return Err(LowerError::Unsupported(format!(
                    "call result passed as {other:?}"
                )));
            }
        }
        self.emitted.insert(result);
        self.emitted.insert(node);
        Ok(())
    }
}
