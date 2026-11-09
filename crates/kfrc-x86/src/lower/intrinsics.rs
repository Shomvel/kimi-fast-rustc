//! Baseline x86-64 integer intrinsic selection. No optional POPCNT/LZCNT/TZCNT
//! instructions are used: their availability is not implied by the host CPU.
use super::*;
use kfrc_son::IntegerIntrinsicPayload as Payload;

impl Lowering<'_> {
    fn intrinsic_binary(
        &mut self,
        opcode: Opcode,
        a: VirtualValue,
        b: VirtualValue,
    ) -> VirtualValue {
        let dst = self.temporary();
        self.push(match opcode {
            Opcode::Add => VirtualInstr::Add {
                dst,
                lhs: a,
                rhs: b,
            },
            Opcode::Sub => VirtualInstr::Sub {
                dst,
                lhs: a,
                rhs: b,
            },
            Opcode::Mul => VirtualInstr::Mul {
                dst,
                lhs: a,
                rhs: b,
            },
            Opcode::BitAnd => VirtualInstr::And {
                dst,
                lhs: a,
                rhs: b,
            },
            Opcode::BitOr => VirtualInstr::Or {
                dst,
                lhs: a,
                rhs: b,
            },
            Opcode::BitXor => VirtualInstr::Xor {
                dst,
                lhs: a,
                rhs: b,
            },
            Opcode::ShiftLeft => VirtualInstr::Shl {
                dst,
                lhs: a,
                rhs: b,
            },
            Opcode::ShiftRightLogical => VirtualInstr::LShr {
                dst,
                lhs: a,
                rhs: b,
            },
            Opcode::ShiftRightArithmetic => VirtualInstr::Shr {
                dst,
                lhs: a,
                rhs: b,
            },
            _ => unreachable!(),
        });
        VirtualValue::Reg(dst)
    }
    fn intrinsic_compare(
        &mut self,
        condition: Condition,
        a: VirtualValue,
        b: VirtualValue,
    ) -> VirtualValue {
        let dst = self.temporary();
        self.push(VirtualInstr::Compare {
            dst,
            condition,
            lhs: a,
            rhs: b,
        });
        VirtualValue::Reg(dst)
    }
    fn unsigned_bits(&mut self, value: VirtualValue, bits: u32) -> VirtualValue {
        if bits == 64 {
            value
        } else {
            self.intrinsic_binary(
                Opcode::BitAnd,
                value,
                VirtualValue::Imm((u64::MAX >> (64 - bits)) as i64),
            )
        }
    }
    fn byte_swap(&mut self, value: VirtualValue, bits: u32) -> VirtualValue {
        if bits == 8 {
            return value;
        }
        let dst = self.temporary();
        if bits == 16 {
            self.instructions.push(LowerInstr::Rotate {
                dst,
                value,
                amount: VirtualValue::Imm(8),
                width: BitWidth::W16,
                right: false,
            });
        } else {
            self.instructions.push(LowerInstr::ByteSwap {
                dst,
                value,
                wide: bits == 64,
            });
        }
        VirtualValue::Reg(dst)
    }
    fn select_integer(
        &mut self,
        condition: VirtualValue,
        yes: VirtualValue,
        no: VirtualValue,
    ) -> VirtualValue {
        // no ^ ((yes ^ no) & -condition), preserving both bit patterns.
        let mask = self.intrinsic_binary(Opcode::Sub, VirtualValue::Imm(0), condition);
        let difference = self.intrinsic_binary(Opcode::BitXor, yes, no);
        let selected = self.intrinsic_binary(Opcode::BitAnd, difference, mask);
        self.intrinsic_binary(Opcode::BitXor, no, selected)
    }
    pub(super) fn emit_integer_intrinsic(
        &mut self,
        node: NodeId,
        args: &[VirtualValue],
    ) -> Result<(), LowerError> {
        use Opcode::*;
        let kind = self.node_kind(self.graph.input(node, 0))?;
        let bits = kind.bits;
        let value = match self.graph.op(node) {
            BitCount => {
                let value = self.unsigned_bits(args[0], bits);
                let mode = self.graph.payload(node) & 3;
                if mode == Payload::ONES {
                    let shifted =
                        self.intrinsic_binary(ShiftRightLogical, value, VirtualValue::Imm(1));
                    let pairs = self.intrinsic_binary(
                        BitAnd,
                        shifted,
                        VirtualValue::Imm(0x5555555555555555),
                    );
                    let x = self.intrinsic_binary(Sub, value, pairs);
                    let lo =
                        self.intrinsic_binary(BitAnd, x, VirtualValue::Imm(0x3333333333333333));
                    let hi = self.intrinsic_binary(ShiftRightLogical, x, VirtualValue::Imm(2));
                    let hi =
                        self.intrinsic_binary(BitAnd, hi, VirtualValue::Imm(0x3333333333333333));
                    let x = self.intrinsic_binary(Add, lo, hi);
                    let hi = self.intrinsic_binary(ShiftRightLogical, x, VirtualValue::Imm(4));
                    let x = self.intrinsic_binary(Add, x, hi);
                    let x = self.intrinsic_binary(BitAnd, x, VirtualValue::Imm(0x0f0f0f0f0f0f0f0f));
                    let x = self.intrinsic_binary(Mul, x, VirtualValue::Imm(0x0101010101010101));
                    self.intrinsic_binary(ShiftRightLogical, x, VirtualValue::Imm(56))
                } else {
                    let dst = self.temporary();
                    let reverse = mode == Payload::LEADING_ZEROS;
                    // A zero input of a ZERO_POISON count yields poison, so
                    // any defined register serves as the fallback; the
                    // scanned value needs no separate constant.
                    let zero_poison = self.graph.payload(node) & Payload::ZERO_POISON != 0;
                    let zero = match value {
                        VirtualValue::Reg(_) if zero_poison => value,
                        _ => VirtualValue::Imm(if reverse { -1 } else { i64::from(bits) }),
                    };
                    self.instructions.push(LowerInstr::BitScanZero {
                        dst,
                        value,
                        zero,
                        reverse,
                    });
                    if reverse {
                        self.intrinsic_binary(
                            Sub,
                            VirtualValue::Imm(i64::from(bits) - 1),
                            VirtualValue::Reg(dst),
                        )
                    } else {
                        VirtualValue::Reg(dst)
                    }
                }
            }
            ByteSwap => self.byte_swap(args[0], bits),
            BitReverse => {
                let mut value = self.unsigned_bits(args[0], bits);
                for (shift, mask) in [
                    (1, 0x5555555555555555i64),
                    (2, 0x3333333333333333),
                    (4, 0x0f0f0f0f0f0f0f0f),
                ] {
                    let hi =
                        self.intrinsic_binary(ShiftRightLogical, value, VirtualValue::Imm(shift));
                    let hi = self.intrinsic_binary(BitAnd, hi, VirtualValue::Imm(mask));
                    let lo = self.intrinsic_binary(BitAnd, value, VirtualValue::Imm(mask));
                    let lo = self.intrinsic_binary(ShiftLeft, lo, VirtualValue::Imm(shift));
                    value = self.intrinsic_binary(BitOr, lo, hi);
                }
                self.byte_swap(value, bits)
            }
            FunnelShift => {
                let right = self.graph.payload(node) & Payload::FUNNEL_RIGHT != 0;
                if self.graph.resolve(self.graph.input(node, 0))
                    == self.graph.resolve(self.graph.input(node, 1))
                {
                    let dst = self.temporary();
                    let width = match bits {
                        8 => BitWidth::W8,
                        16 => BitWidth::W16,
                        32 => BitWidth::W32,
                        64 => BitWidth::W64,
                        _ => unreachable!(),
                    };
                    self.instructions.push(LowerInstr::Rotate {
                        dst,
                        value: args[0],
                        amount: args[2],
                        width,
                        right,
                    });
                    VirtualValue::Reg(dst)
                } else {
                    let hi = self.unsigned_bits(args[0], bits);
                    let lo = self.unsigned_bits(args[1], bits);
                    let amount = self.intrinsic_binary(
                        BitAnd,
                        args[2],
                        VirtualValue::Imm(i64::from(bits) - 1),
                    );
                    let complement =
                        self.intrinsic_binary(Sub, VirtualValue::Imm(i64::from(bits)), amount);
                    let (base, other) = if right { (lo, hi) } else { (hi, lo) };
                    let primary = self.intrinsic_binary(
                        if right { ShiftRightLogical } else { ShiftLeft },
                        base,
                        amount,
                    );
                    let secondary = self.intrinsic_binary(
                        if right { ShiftLeft } else { ShiftRightLogical },
                        other,
                        complement,
                    );
                    let secondary = if bits == 64 {
                        let nonzero =
                            self.intrinsic_compare(Condition::NotEq, amount, VirtualValue::Imm(0));
                        let mask = self.intrinsic_binary(Sub, VirtualValue::Imm(0), nonzero);
                        self.intrinsic_binary(BitAnd, secondary, mask)
                    } else {
                        secondary
                    };
                    self.intrinsic_binary(BitOr, primary, secondary)
                }
            }
            SaturatingAdd | SaturatingSub => {
                let subtract = self.graph.op(node) == SaturatingSub;
                let sum = self.intrinsic_binary(if subtract { Sub } else { Add }, args[0], args[1]);
                let canonical = self.temporary();
                self.canonicalize(canonical, sum, kind);
                let result = VirtualValue::Reg(canonical);
                if !kind.signed {
                    let overflow = self.intrinsic_compare(
                        Condition::UnsignedBelow,
                        if subtract { args[0] } else { result },
                        if subtract { args[1] } else { args[0] },
                    );
                    let limit = VirtualValue::Imm(if subtract {
                        0
                    } else {
                        (u64::MAX >> (64 - bits)) as i64
                    });
                    self.select_integer(overflow, limit, result)
                } else {
                    let a_xor_b = self.intrinsic_binary(BitXor, args[0], args[1]);
                    let signs = if subtract {
                        a_xor_b
                    } else {
                        self.intrinsic_binary(BitXor, a_xor_b, VirtualValue::Imm(-1))
                    };
                    let changed = self.intrinsic_binary(BitXor, args[0], result);
                    let overflow_bits = self.intrinsic_binary(BitAnd, signs, changed);
                    let overflow = self.intrinsic_compare(
                        Condition::SignedLess,
                        overflow_bits,
                        VirtualValue::Imm(0),
                    );
                    let sign =
                        self.intrinsic_binary(ShiftRightArithmetic, args[0], VirtualValue::Imm(63));
                    let max = (u64::MAX >> (65 - bits)) as i64;
                    let limit = self.intrinsic_binary(BitXor, VirtualValue::Imm(max), sign);
                    self.select_integer(overflow, limit, result)
                }
            }
            _ => unreachable!(),
        };
        let output = self.node_kind(node)?;
        // Count results are already in 0..=64, so need no u32 mask.
        if self.graph.op(node) == BitCount {
            self.push(VirtualInstr::Mov {
                dst: register(node),
                src: value,
            });
        } else {
            self.canonicalize(register(node), value, output);
        }
        Ok(())
    }
}
