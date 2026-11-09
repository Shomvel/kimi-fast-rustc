//! Calling-convention value materialization and frame storage helpers.
use super::*;

impl Lowering<'_> {
    pub(super) fn allocate_slot(
        &mut self,
        destination: VirtualReg,
        ty: TyId,
    ) -> Result<(), LowerError> {
        let layout = self.graph.oracle.size_align(ty).ok_or_else(|| {
            LowerError::Unsupported(format!(
                "unsized value `{}`",
                self.graph.oracle.describe(ty)
            ))
        })?;
        let slot = self.stack_layouts.len();
        self.stack_layouts.push(layout);
        self.instructions.push(LowerInstr::StackAddress {
            dst: destination,
            slot,
        });
        Ok(())
    }

    pub(super) fn offset_address(&mut self, address: VirtualValue, offset: u64) -> VirtualValue {
        if offset == 0 {
            return address;
        }
        let dst = self.temporary();
        self.push(VirtualInstr::Add {
            dst,
            lhs: address,
            rhs: VirtualValue::Imm(offset as i64),
        });
        VirtualValue::Reg(dst)
    }

    pub(super) fn load_field(
        &mut self,
        address: VirtualValue,
        offset: u64,
        ty: TyId,
    ) -> Result<VirtualValue, LowerError> {
        let address = self.offset_address(address, offset);
        let kind = self.scalar_kind(ty)?;
        let dst = self.temporary();
        let raw = self.temporary();
        self.instructions.push(LowerInstr::Load {
            dst: raw,
            address,
            bytes: (kind.bits / 8) as u8,
        });
        self.canonicalize(dst, VirtualValue::Reg(raw), kind);
        Ok(VirtualValue::Reg(dst))
    }

    pub(super) fn store_field(
        &mut self,
        address: VirtualValue,
        offset: u64,
        ty: TyId,
        value: VirtualValue,
    ) -> Result<(), LowerError> {
        let address = self.offset_address(address, offset);
        let kind = self.scalar_kind(ty)?;
        self.instructions.push(LowerInstr::Store {
            address,
            src: value,
            bytes: (kind.bits / 8) as u8,
        });
        Ok(())
    }

    pub(super) fn copy_bytes(
        &mut self,
        destination: VirtualValue,
        source: VirtualValue,
        size: u64,
    ) {
        if size > self.options.max_unrolled_memory_bytes {
            // Use the regular virtual call path: allocation preserves live
            // values across SysV clobbers, including the hidden return pointer.
            // memmove also handles Layout graphs whose source aliases sret.
            let result = self.temporary();
            self.push(VirtualInstr::Call {
                dst: result,
                target: "memmove".into(),
                args: vec![destination, source, VirtualValue::Imm(size as i64)],
            });
            return;
        }
        let mut offset = 0;
        while offset < size {
            let bytes = match size - offset {
                8.. => 8,
                4..=7 => 4,
                2..=3 => 2,
                _ => 1,
            };
            let src_address = self.offset_address(source, offset);
            let dst_address = self.offset_address(destination, offset);
            let value = self.temporary();
            self.instructions.push(LowerInstr::Load {
                dst: value,
                address: src_address,
                bytes,
            });
            self.instructions.push(LowerInstr::Store {
                address: dst_address,
                src: VirtualValue::Reg(value),
                bytes,
            });
            offset += u64::from(bytes);
        }
    }

    pub(super) fn direct_bits(&self, ty: TyId) -> Result<u8, LowerError> {
        let layout = self
            .graph
            .oracle
            .size_align(ty)
            .ok_or_else(|| LowerError::Unsupported("unsized direct argument".to_owned()))?;
        match layout.size {
            1 | 2 | 4 | 8 => Ok(layout.size as u8),
            _ => Err(LowerError::Unsupported(format!(
                "direct aggregate of {} bytes",
                layout.size
            ))),
        }
    }

    pub(super) fn emit_parameters(&mut self) -> Result<(), LowerError> {
        let graph = self.graph;
        let mut passed = 0usize;
        if graph.abi.ret == PassMode::Indirect {
            let address = self.temporary();
            self.push(VirtualInstr::LoadArg {
                dst: address,
                index: 0,
            });
            self.return_address = Some(address);
            passed = 1;
        }
        let plan = super::vector::argument_plan(&graph.abi)?;
        for (index, mode) in graph.abi.args.iter().enumerate() {
            let locations = &plan.integers[index];
            let count = locations.len();
            passed = locations.first().copied().unwrap_or(passed);
            let Some(parameter) = self
                .schedule
                .projections
                .get(&(graph.start, 2 + index as u32))
                .copied()
            else {
                passed += count;
                continue;
            };
            self.emitted.insert(parameter);
            let ty = graph
                .rust_type(parameter)
                .ok_or_else(|| LowerError::Malformed("parameter has no Rust type".to_owned()))?;
            match mode {
                PassMode::Vector { bytes: 16 } => {
                    let location = plan.vectors[index].ok_or_else(|| {
                        LowerError::Malformed("vector argument has no location".into())
                    })?;
                    self.receive_vector_argument(parameter, ty, location)?;
                }
                PassMode::Ignore => self.push(VirtualInstr::Mov {
                    dst: register(parameter),
                    src: VirtualValue::Imm(0),
                }),
                PassMode::Direct if self.scalar_kind(ty).is_ok() => {
                    let raw = self.temporary();
                    self.push(VirtualInstr::LoadArg {
                        dst: raw,
                        index: passed,
                    });
                    let kind = self.node_kind(parameter)?;
                    self.canonicalize(register(parameter), VirtualValue::Reg(raw), kind);
                }
                PassMode::Direct => {
                    self.allocate_slot(register(parameter), ty)?;
                    let raw = self.temporary();
                    self.push(VirtualInstr::LoadArg {
                        dst: raw,
                        index: passed,
                    });
                    let bytes = self.direct_bits(ty)?;
                    self.instructions.push(LowerInstr::Store {
                        address: VirtualValue::Reg(register(parameter)),
                        src: VirtualValue::Reg(raw),
                        bytes,
                    });
                }
                PassMode::Indirect => {
                    self.push(VirtualInstr::LoadArg {
                        dst: register(parameter),
                        index: passed,
                    });
                }
                PassMode::Cast(cast) => {
                    let mut values = Vec::new();
                    for location in locations.iter().skip(usize::from(cast.pad_i32)) {
                        let raw = self.temporary();
                        self.push(VirtualInstr::LoadArg {
                            dst: raw,
                            index: *location,
                        });
                        values.push(VirtualValue::Reg(raw));
                    }
                    self.receive_cast(parameter, *cast, &values)?;
                }
                PassMode::Pair => {
                    let fields = *graph.oracle.abi_pairs.get(&ty).ok_or_else(|| {
                        LowerError::Unsupported(format!(
                            "missing ScalarPair layout for `{}`",
                            graph.oracle.describe(ty)
                        ))
                    })?;
                    self.allocate_slot(register(parameter), ty)?;
                    for (lane, field) in fields.iter().enumerate() {
                        let raw = self.temporary();
                        self.push(VirtualInstr::LoadArg {
                            dst: raw,
                            index: locations[lane],
                        });
                        self.store_field(
                            VirtualValue::Reg(register(parameter)),
                            field.offset,
                            field.ty,
                            VirtualValue::Reg(raw),
                        )?;
                    }
                }
                other => {
                    return Err(LowerError::Unsupported(format!(
                        "parameter passed as {other:?}"
                    )));
                }
            }
            passed += count;
        }
        Ok(())
    }
}
