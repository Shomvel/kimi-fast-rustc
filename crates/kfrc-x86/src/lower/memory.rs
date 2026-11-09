//! Memory lowering with allocated values and reserved scratch registers.
use super::*;

/// Keep addressable locals in memory, but allocate computed values separately.
/// The pool excludes argument, result and scratch registers used below.
pub(super) fn compile_memory_function(
    graph: &FunctionGraph,
    instructions: Vec<LowerInstr>,
    virtual_count: u32,
    stack_layouts: &[SizeAlign],
) -> Result<MachineFunction, LowerError> {
    let mut function = MachineFunction::new(&graph.symbol);
    let facts: Vec<_> = instructions
        .iter()
        .map(super::memory_liveness::facts)
        .collect();
    let allocation = crate::regalloc::memory::MemoryAllocation::new(&facts, virtual_count)?;
    function.callee_saved = allocation.callee_saved.clone();
    let mut bytes = u64::from(allocation.spill_slots) * 8;
    let mut addresses = Vec::new();
    for layout in stack_layouts {
        if layout.align == 0 || !layout.align.is_power_of_two() {
            return Err(LowerError::Unsupported(format!(
                "stack alignment {}",
                layout.align
            )));
        }
        let overaligned = layout.align > 16;
        let alignment = if overaligned {
            i32::try_from(layout.align).map_err(|_| {
                LowerError::Unsupported(format!(
                    "stack alignment {} exceeds x86 immediate range",
                    layout.align
                ))
            })?
        } else {
            1
        };
        bytes = bytes
            .checked_add(layout.size.max(1))
            .and_then(|bytes| bytes.checked_add(layout.align - 1))
            .map(|bytes| {
                if overaligned {
                    // Rbp is guaranteed only 16-byte alignment. Give this slot
                    // an independent [start, start + size + align - 1) region;
                    // rounding its address upward always fits within that region.
                    bytes
                } else {
                    bytes & !(layout.align - 1)
                }
            })
            .ok_or_else(|| {
                LowerError::Unsupported("stack frame exceeds address space".to_owned())
            })?;
        let offset = -i32::try_from(bytes)
            .map_err(|_| LowerError::Unsupported("stack frame exceeds i32".to_owned()))?;
        addresses.push((offset, alignment));
    }
    let argument_base = u32::try_from(bytes.div_ceil(8))
        .map_err(|_| LowerError::Unsupported("stack frame exceeds slot space".to_owned()))?;
    // The shared machine frame builder reserves outgoing stack arguments from
    // StoreArg instructions, below locals and any saved registers.
    function.stack_slots = argument_base
        .checked_add(6)
        .ok_or_else(|| LowerError::Unsupported("stack frame exceeds slot space".to_owned()))?;
    // Capture argument registers before canonicalization uses rcx/rdx.
    for (index, reg) in Reg::ARGUMENTS.into_iter().enumerate() {
        function.instructions.push(MachineInstr::Store {
            slot: StackSlot(argument_base + index as u32),
            src: reg,
        });
    }
    let load = |output: &mut Vec<MachineInstr>, value: VirtualValue, dst: Reg| {
        allocation.load(output, value, dst);
    };
    let save = |output: &mut Vec<MachineInstr>, dst: VirtualReg, src: Reg| {
        allocation.save(output, dst, src);
    };
    for instruction in instructions {
        match instruction {
            LowerInstr::StackAddress { dst, slot } => {
                let (offset, alignment) = *addresses
                    .get(slot)
                    .ok_or_else(|| LowerError::Malformed(format!("missing stack slot {slot}")))?;
                function.instructions.push(MachineInstr::StackAddress {
                    dst: Reg::Rax,
                    offset,
                });
                if alignment > 1 {
                    // Align within the padded slot rather than realigning Rsp.
                    // Spill slots, incoming stack arguments and call alignment
                    // retain the existing frame-pointer-relative ABI contract.
                    function.instructions.push(MachineInstr::Add {
                        dst: Reg::Rax,
                        src: MachineOperand::Imm(alignment - 1),
                    });
                    function.instructions.push(MachineInstr::And {
                        dst: Reg::Rax,
                        src: MachineOperand::Imm(-alignment),
                    });
                }
                save(&mut function.instructions, dst, Reg::Rax);
            }
            LowerInstr::SymbolAddress {
                dst,
                symbol,
                addend,
            } => {
                let index = match function
                    .symbols
                    .iter()
                    .position(|existing| *existing == symbol)
                {
                    Some(index) => index,
                    None => {
                        function.symbols.push(symbol);
                        function.symbols.len() - 1
                    }
                };
                function.instructions.push(MachineInstr::SymbolAddress {
                    dst: Reg::Rax,
                    symbol: index as u32,
                    addend,
                });
                save(&mut function.instructions, dst, Reg::Rax);
            }
            LowerInstr::ThreadLocalAddress { dst, symbol } => {
                let index = match function.symbols.iter().position(|entry| *entry == symbol) {
                    Some(index) => index,
                    None => {
                        function.symbols.push(symbol);
                        function.symbols.len() - 1
                    }
                };
                function
                    .instructions
                    .push(MachineInstr::ThreadLocalAddress {
                        dst: Reg::Rax,
                        symbol: index as u32,
                    });
                save(&mut function.instructions, dst, Reg::Rax);
            }
            LowerInstr::LoadVector { dst, address } => {
                load(&mut function.instructions, address, Reg::R10);
                function.instructions.push(MachineInstr::LoadVector {
                    dst,
                    address: Reg::R10,
                });
            }
            LowerInstr::StoreVector { address, src } => {
                load(&mut function.instructions, address, Reg::R10);
                function.instructions.push(MachineInstr::StoreVector {
                    address: Reg::R10,
                    src,
                });
            }
            LowerInstr::Load {
                dst,
                address,
                bytes,
            } => {
                load(&mut function.instructions, address, Reg::R10);
                function.instructions.push(MachineInstr::LoadIndirect {
                    dst: Reg::Rax,
                    address: Reg::R10,
                    bytes,
                });
                save(&mut function.instructions, dst, Reg::Rax);
            }
            LowerInstr::Store {
                address,
                src,
                bytes,
            } => {
                load(&mut function.instructions, address, Reg::R10);
                load(&mut function.instructions, src, Reg::R11);
                function.instructions.push(MachineInstr::StoreIndirect {
                    address: Reg::R10,
                    src: Reg::R11,
                    bytes,
                });
            }
            LowerInstr::MulOverflow {
                dst,
                lhs,
                rhs,
                signed,
                bits,
            } => {
                load(&mut function.instructions, lhs, Reg::R10);
                load(&mut function.instructions, rhs, Reg::R11);
                function.instructions.push(MachineInstr::MulOverflow {
                    dst: Reg::R10,
                    lhs: Reg::R10,
                    rhs: Reg::R11,
                    signed,
                    bits,
                });
                save(&mut function.instructions, dst, Reg::R10);
            }
            LowerInstr::AbiCall {
                results,
                target,
                args,
                vector_args,
                vector_result,
            } => {
                for (location, value) in args {
                    if location < 6 {
                        load(&mut function.instructions, value, Reg::ARGUMENTS[location]);
                    } else {
                        load(&mut function.instructions, value, Reg::Rax);
                        function.instructions.push(MachineInstr::StoreArg {
                            src: Reg::Rax,
                            index: location,
                        });
                    }
                }
                for (dst, address) in vector_args {
                    load(&mut function.instructions, address, Reg::R10);
                    function.instructions.push(MachineInstr::LoadVector {
                        dst,
                        address: Reg::R10,
                    });
                }
                match target {
                    CallDestination::Direct(target) => {
                        let symbol = match function
                            .symbols
                            .iter()
                            .position(|existing| *existing == target)
                        {
                            Some(index) => index,
                            None => {
                                function.symbols.push(target);
                                function.symbols.len() - 1
                            }
                        };
                        function.instructions.push(MachineInstr::Call {
                            target: symbol as u32,
                        });
                    }
                    CallDestination::Indirect(target) => {
                        load(&mut function.instructions, target, Reg::R11);
                        function
                            .instructions
                            .push(MachineInstr::CallIndirect { target: Reg::R11 });
                    }
                }
                for (value, register) in results.into_iter().zip([Reg::Rax, Reg::Rdx]) {
                    save(&mut function.instructions, value, register);
                }
                if let Some(address) = vector_result {
                    load(&mut function.instructions, address, Reg::R10);
                    function.instructions.push(MachineInstr::StoreVector {
                        address: Reg::R10,
                        src: Xmm(0),
                    });
                }
            }
            LowerInstr::ReturnPair { first, second } => {
                load(&mut function.instructions, first, Reg::Rax);
                load(&mut function.instructions, second, Reg::Rdx);
                function.instructions.push(MachineInstr::Ret);
            }
            LowerInstr::CallIndirect { dst, target, args } => {
                for (index, value) in args.into_iter().enumerate() {
                    if index < 6 {
                        load(&mut function.instructions, value, Reg::ARGUMENTS[index]);
                    } else {
                        load(&mut function.instructions, value, Reg::Rax);
                        function.instructions.push(MachineInstr::StoreArg {
                            src: Reg::Rax,
                            index,
                        });
                    }
                }
                load(&mut function.instructions, target, Reg::R11);
                function
                    .instructions
                    .push(MachineInstr::CallIndirect { target: Reg::R11 });
                save(&mut function.instructions, dst, Reg::Rax);
            }
            LowerInstr::CallPair {
                first,
                second,
                target,
                args,
            } => {
                for (index, value) in args.into_iter().enumerate() {
                    if index < 6 {
                        load(&mut function.instructions, value, Reg::ARGUMENTS[index]);
                    } else {
                        load(&mut function.instructions, value, Reg::Rax);
                        function.instructions.push(MachineInstr::StoreArg {
                            src: Reg::Rax,
                            index,
                        });
                    }
                }
                match target {
                    CallDestination::Direct(target) => {
                        let symbol = match function
                            .symbols
                            .iter()
                            .position(|existing| *existing == target)
                        {
                            Some(index) => index,
                            None => {
                                function.symbols.push(target);
                                function.symbols.len() - 1
                            }
                        };
                        function.instructions.push(MachineInstr::Call {
                            target: symbol as u32,
                        });
                    }
                    CallDestination::Indirect(target) => {
                        load(&mut function.instructions, target, Reg::R11);
                        function
                            .instructions
                            .push(MachineInstr::CallIndirect { target: Reg::R11 });
                    }
                }
                save(&mut function.instructions, first, Reg::Rax);
                save(&mut function.instructions, second, Reg::Rdx);
            }
            LowerInstr::Scalar(instruction) => {
                use VirtualInstr::*;
                match instruction {
                    LoadArg { dst, index } => {
                        function.instructions.push(if index < 6 {
                            MachineInstr::Load {
                                dst: Reg::Rax,
                                slot: StackSlot(argument_base + index as u32),
                            }
                        } else {
                            MachineInstr::LoadArg {
                                dst: Reg::Rax,
                                index,
                            }
                        });
                        save(&mut function.instructions, dst, Reg::Rax);
                    }
                    Mov { dst, src } => {
                        allocation.copy(&mut function.instructions, dst, src);
                    }
                    Neg { dst, src } | Not { dst, src } => {
                        load(&mut function.instructions, src, Reg::Rax);
                        function
                            .instructions
                            .push(if matches!(instruction, Neg { .. }) {
                                MachineInstr::Neg { dst: Reg::Rax }
                            } else {
                                MachineInstr::Not { dst: Reg::Rax }
                            });
                        save(&mut function.instructions, dst, Reg::Rax);
                    }
                    Add { dst, lhs, rhs }
                    | Sub { dst, lhs, rhs }
                    | Mul { dst, lhs, rhs }
                    | And { dst, lhs, rhs }
                    | Or { dst, lhs, rhs }
                    | Xor { dst, lhs, rhs } => {
                        load(&mut function.instructions, lhs, Reg::R10);
                        load(&mut function.instructions, rhs, Reg::R11);
                        let (dst_reg, src) = (Reg::R10, MachineOperand::Reg(Reg::R11));
                        function.instructions.push(match instruction {
                            Add { .. } => MachineInstr::Add { dst: dst_reg, src },
                            Sub { .. } => MachineInstr::Sub { dst: dst_reg, src },
                            Mul { .. } => MachineInstr::IMul { dst: dst_reg, src },
                            And { .. } => MachineInstr::And { dst: dst_reg, src },
                            Or { .. } => MachineInstr::Or { dst: dst_reg, src },
                            _ => MachineInstr::Xor { dst: dst_reg, src },
                        });
                        save(&mut function.instructions, dst, Reg::R10);
                    }
                    Shl { dst, lhs, rhs } | Shr { dst, lhs, rhs } | LShr { dst, lhs, rhs } => {
                        load(&mut function.instructions, lhs, Reg::R10);
                        load(&mut function.instructions, rhs, Reg::Rcx);
                        function.instructions.push(match instruction {
                            Shl { .. } => MachineInstr::Shl { dst: Reg::R10 },
                            Shr { .. } => MachineInstr::Shr { dst: Reg::R10 },
                            _ => MachineInstr::LShr { dst: Reg::R10 },
                        });
                        save(&mut function.instructions, dst, Reg::R10);
                    }
                    Div { dst, lhs, rhs }
                    | Rem { dst, lhs, rhs }
                    | UDiv { dst, lhs, rhs }
                    | URem { dst, lhs, rhs } => {
                        load(&mut function.instructions, lhs, Reg::Rax);
                        load(&mut function.instructions, rhs, Reg::R11);
                        if matches!(instruction, Div { .. } | Rem { .. }) {
                            function.instructions.push(MachineInstr::Cqo);
                            function.instructions.push(MachineInstr::IDiv {
                                src: MachineOperand::Reg(Reg::R11),
                            });
                        } else {
                            function.instructions.push(MachineInstr::Xor {
                                dst: Reg::Rdx,
                                src: MachineOperand::Reg(Reg::Rdx),
                            });
                            function.instructions.push(MachineInstr::UDiv {
                                src: MachineOperand::Reg(Reg::R11),
                            });
                        }
                        save(
                            &mut function.instructions,
                            dst,
                            if matches!(instruction, Rem { .. } | URem { .. }) {
                                Reg::Rdx
                            } else {
                                Reg::Rax
                            },
                        );
                    }
                    Compare {
                        dst,
                        condition,
                        lhs,
                        rhs,
                    } => {
                        load(&mut function.instructions, lhs, Reg::R10);
                        load(&mut function.instructions, rhs, Reg::R11);
                        function.instructions.push(MachineInstr::Cmp {
                            lhs: Reg::R10,
                            rhs: MachineOperand::Reg(Reg::R11),
                        });
                        function.instructions.push(MachineInstr::SetCc {
                            condition,
                            dst: Reg::Rax,
                        });
                        save(&mut function.instructions, dst, Reg::Rax);
                    }
                    Test { lhs, rhs } => {
                        load(&mut function.instructions, lhs, Reg::R10);
                        if rhs == VirtualValue::Imm(0) {
                            function
                                .instructions
                                .push(MachineInstr::Test { reg: Reg::R10 });
                        } else {
                            load(&mut function.instructions, rhs, Reg::R11);
                            function.instructions.push(MachineInstr::Cmp {
                                lhs: Reg::R10,
                                rhs: MachineOperand::Reg(Reg::R11),
                            });
                        }
                    }
                    Load { dst, slot } => {
                        function.instructions.push(MachineInstr::Load {
                            dst: Reg::Rax,
                            slot,
                        });
                        save(&mut function.instructions, dst, Reg::Rax);
                    }
                    Store { slot, src } => {
                        load(&mut function.instructions, src, Reg::Rax);
                        function.instructions.push(MachineInstr::Store {
                            slot,
                            src: Reg::Rax,
                        });
                    }
                    Label(label) => function.instructions.push(MachineInstr::Label(label)),
                    Jmp(label) => function.instructions.push(MachineInstr::Jmp(label)),
                    Jcc { condition, target } => function
                        .instructions
                        .push(MachineInstr::Jcc { condition, target }),
                    Call { dst, target, args } => {
                        for (index, value) in args.into_iter().enumerate() {
                            if index < 6 {
                                load(&mut function.instructions, value, Reg::ARGUMENTS[index]);
                            } else {
                                load(&mut function.instructions, value, Reg::Rax);
                                function.instructions.push(MachineInstr::StoreArg {
                                    src: Reg::Rax,
                                    index,
                                });
                            }
                        }
                        let symbol = match function
                            .symbols
                            .iter()
                            .position(|existing| *existing == target)
                        {
                            Some(index) => index,
                            None => {
                                function.symbols.push(target);
                                function.symbols.len() - 1
                            }
                        };
                        function.instructions.push(MachineInstr::Call {
                            target: symbol as u32,
                        });
                        save(&mut function.instructions, dst, Reg::Rax);
                    }
                    Return(value) => {
                        load(&mut function.instructions, value, Reg::Rax);
                        function.instructions.push(MachineInstr::Ret);
                    }
                    Nop | Phi { .. } => {}
                }
            }
        }
    }
    Ok(function)
}
