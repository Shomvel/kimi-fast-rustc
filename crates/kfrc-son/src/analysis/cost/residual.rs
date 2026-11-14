//! Estimate operations that survive semantic packaging. This is a versioned
//! profitability model, not an instruction selector or a cycle predictor.
use super::{CostKind, CostModel, InstructionCost};
use crate::{FunctionGraph, NodeId, Opcode, ValueType};
use kfrc_body::{PassMode, TypeOracle};

#[derive(Clone, Copy, Debug, Default)]
pub struct ResidualCostModel;

impl CostModel for ResidualCostModel {
    fn name(&self) -> &'static str {
        "residual-ir-v1"
    }

    fn instruction_cost(
        &self,
        graph: &FunctionGraph,
        node: NodeId,
        kind: CostKind,
    ) -> InstructionCost {
        use Opcode::*;
        let node = graph.resolve(node);
        let units = match graph.op(node) {
            // These express SSA packaging or source requirements. Their
            // runtime producers/consumers carry the cost of materialization.
            Start | Projection | Region | Phi | VariantWitness | Assume | RequireDefined
            | SlotLive | SlotDead | Undefined | Const | ConstRepr | PointerCast | Aggregate
            | Repeat | Field | InsertField | MakeVariant | VariantField | WidePtr | Opaque
            | TransmuteValue | Return | Unreachable => 0,
            // The callee's Return and entry ABI disappear on substitution.
            // Nested call boundaries remain, including argument preparation.
            Call | CallIndirect => {
                return if kind == CostKind::CodeSize {
                    call_units(graph, node).map_or(InstructionCost::Unknown, InstructionCost::Known)
                } else {
                    InstructionCost::Unknown
                };
            }
            Hole | TraitCall | ThreadLocalAddress | Switch => return InstructionCost::Unknown,
            Jump | If | VariantOf | Discriminant | LocalSlot | StackSlot | SymbolAddress
            | FieldAddr | VariantFieldAddr | IndexAddr | PtrOffset => 1,
            VariantSwitch => graph.variant_switches[graph.payload(node) as usize]
                .len()
                .max(1) as u64,
            Load | LoadValue => {
                let ValueType::Tuple(outputs) = graph.value_type(node) else {
                    return InstructionCost::Unknown;
                };
                return memory_units(graph, outputs.get(2), kind);
            }
            Store | StoreValue => {
                return memory_units(
                    graph,
                    Some(graph.value_type(graph.resolve(graph.input(node, 2)))),
                    kind,
                );
            }
            SetVariant => 1,
            CopyNonOverlapping => {
                // A copy has a bounded code sequence even when its runtime
                // length is unknown. Do not invent its dynamic work/latency.
                return if kind == CostKind::CodeSize {
                    InstructionCost::Known(7)
                } else {
                    InstructionCost::Unknown
                };
            }
            Add | Sub | BitAnd | BitOr | BitXor | Negate | BitNot | Compare | IntCast
            | ShiftLeft | ShiftRightArithmetic | ShiftRightLogical | ByteSwap | BitCount => 1,
            Mul => {
                if kind == CostKind::CodeSize {
                    1
                } else {
                    3
                }
            }
            DivSigned | DivUnsigned | RemSigned | RemUnsigned => {
                if kind == CostKind::CodeSize {
                    2
                } else {
                    16
                }
            }
            MulOverflow | FunnelShift | SaturatingAdd | SaturatingSub => 3,
            BitReverse => 12,
        };
        InstructionCost::Known(units)
    }
}

fn memory_units(graph: &FunctionGraph, ty: Option<&ValueType>, kind: CostKind) -> InstructionCost {
    let units = ty.and_then(|ty| lanes(graph, ty));
    let scale = if kind == CostKind::CodeSize { 1 } else { 2 };
    units
        .and_then(|units| units.checked_mul(scale))
        .map_or(InstructionCost::Unknown, InstructionCost::Known)
}

fn lanes(graph: &FunctionGraph, ty: &ValueType) -> Option<u64> {
    let (ValueType::Rust(ty) | ValueType::LayoutValue(ty)) = ty else {
        return None;
    };
    Some(graph.oracle.size_align(*ty)?.size.div_ceil(8))
}

fn registers(mode: PassMode) -> Option<u64> {
    match mode {
        PassMode::Ignore => Some(0),
        PassMode::Direct | PassMode::Vector { .. } | PassMode::Indirect => Some(1),
        PassMode::Pair => Some(2),
        PassMode::Cast(cast) => {
            Some(cast.lanes.iter().flatten().count() as u64 + u64::from(cast.pad_i32))
        }
        PassMode::IndirectOnStack => None,
    }
}

fn call_units(graph: &FunctionGraph, call: NodeId) -> Option<u64> {
    let (abi, first_argument) = if graph.op(call) == Opcode::Call {
        (&graph.callees[graph.payload(call) as usize].abi, 1)
    } else {
        (&graph.call_abis[graph.payload(call) as usize], 2)
    };
    let mut units = 1u64.checked_add(registers(abi.ret)?)?;
    for (index, &mode) in abi.args.iter().enumerate() {
        units = units.checked_add(registers(mode)?)?;
        if mode == PassMode::Indirect {
            let argument = graph.resolve(*graph.inputs(call).get(first_argument + index)?);
            // Indirect semantic arguments need a private ABI snapshot unless
            // later location forwarding removes it. Keep that transport visible.
            units = units.checked_add(lanes(graph, graph.value_type(argument))?)?;
        }
    }
    Some(units)
}
