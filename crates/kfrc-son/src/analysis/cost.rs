//! Read-only, target-independent IR costs, inspired by LLVM's TTI interface.
//!
//! Units are relative estimates, not encoded bytes, CPU cycles or execution
//! counts. In particular, a sum of reciprocal-throughput costs is static work,
//! not the latency of a graph. Layout, register allocation, frequencies and
//! cache/branch behavior need separate models. Edits invalidate summaries.
use kfrc_body::{TypeOracle, TypeShape};

use crate::{FunctionGraph, NodeId, Opcode, ValueType};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CostKind {
    CodeSize,
    ReciprocalThroughput,
    Latency,
}

/// Unknown is deliberately not ordered against Known. An unmodeled operation
/// must never win a profitability comparison by being treated as free.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstructionCost {
    Known(u64),
    Unknown,
}

pub trait CostModel {
    fn name(&self) -> &'static str;
    fn instruction_cost(
        &self,
        graph: &FunctionGraph,
        node: NodeId,
        kind: CostKind,
    ) -> InstructionCost;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CostSum {
    pub known_units: u64,
    pub unknown_nodes: usize,
}

impl CostSum {
    pub fn total(self) -> Option<u64> {
        (self.unknown_nodes == 0).then_some(self.known_units)
    }

    fn add(&mut self, cost: InstructionCost) {
        match cost {
            InstructionCost::Known(value) => match self.known_units.checked_add(value) {
                Some(total) => self.known_units = total,
                None => self.unknown_nodes += 1,
            },
            InstructionCost::Unknown => self.unknown_nodes += 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GraphCost {
    /// Structural compiler work, distinct from generated code costs.
    pub nodes: usize,
    pub input_edges: usize,
    pub code_size: CostSum,
    pub static_work: CostSum,
}

pub fn estimate_graph(graph: &FunctionGraph, model: &(impl CostModel + ?Sized)) -> GraphCost {
    let mut result = GraphCost::default();
    for node in graph.ids() {
        result.nodes += 1;
        result.input_edges += graph.inputs(node).len();
        result
            .code_size
            .add(model.instruction_cost(graph, node, CostKind::CodeSize));
        result
            .static_work
            .add(model.instruction_cost(graph, node, CostKind::ReciprocalThroughput));
    }
    result
}

/// A deliberately small, versioned seed model. Scalar add is one unit.
/// Weights express qualitative relative costs, not a calibrated CPU model.
/// Aggregate transport, unresolved types and unknown operations remain unknown.
#[derive(Clone, Copy, Debug, Default)]
pub struct GenericCostModel;

impl CostModel for GenericCostModel {
    fn name(&self) -> &'static str {
        "generic-ir-v1"
    }

    fn instruction_cost(
        &self,
        graph: &FunctionGraph,
        node: NodeId,
        kind: CostKind,
    ) -> InstructionCost {
        use Opcode::*;
        let node = graph.resolve(node);
        let op = graph.op(node);
        // Compile-time evidence and structural edges emit no independent work.
        // Phi moves and aggregate projections, unlike control projections, can
        // survive lowering and cannot be declared free here.
        let costs = match op {
            Start | Region | VariantWitness | Assume | RequireDefined | SlotLive | SlotDead
            | Undefined | PointerCast => (0, 0, 0),
            Projection
                if matches!(
                    graph.value_type(node),
                    ValueType::Control | ValueType::Effect
                ) =>
            {
                (0, 0, 0)
            }
            Projection if graph.op(graph.resolve(graph.input(node, 0))) == Start => (0, 0, 0),
            Const if scalar(graph, node) => (1, 1, 1),
            Add | Sub | BitAnd | BitOr | BitXor | Negate | BitNot | Compare | IntCast
                if scalar_inputs(graph, node) =>
            {
                (1, 1, 1)
            }
            Mul if scalar_inputs(graph, node) => (1, 2, 3),
            DivSigned | DivUnsigned | RemSigned | RemUnsigned if scalar_inputs(graph, node) => {
                (2, 8, 16)
            }
            ShiftLeft | ShiftRightArithmetic | ShiftRightLogical if scalar_inputs(graph, node) => {
                let immediate = graph.op(graph.resolve(graph.input(node, 1))) == Const;
                if immediate { (1, 1, 1) } else { (2, 2, 2) }
            }
            ByteSwap if scalar_inputs(graph, node) => (1, 1, 1),
            FieldAddr | VariantFieldAddr | IndexAddr | PtrOffset | SymbolAddress | StackSlot
            | LocalSlot => (1, 1, 1),
            Load | Store => (1, 2, 4),
            LoadValue if scalar_result(graph, node) => (1, 2, 4),
            StoreValue if scalar(graph, graph.input(node, 2)) => (1, 2, 1),
            Jump | If | Return => (1, 1, 1),
            // A call's code-size estimate covers the call boundary only; its
            // dynamic callee work/latency is unknown, never an assumed constant.
            Call | CallIndirect if kind == CostKind::CodeSize => (4, 0, 0),
            _ => return InstructionCost::Unknown,
        };
        InstructionCost::Known(match kind {
            CostKind::CodeSize => costs.0,
            CostKind::ReciprocalThroughput => costs.1,
            CostKind::Latency => costs.2,
        })
    }
}

fn scalar_inputs(graph: &FunctionGraph, node: NodeId) -> bool {
    graph.inputs(node).iter().all(|&input| scalar(graph, input)) && scalar(graph, node)
}

fn scalar_result(graph: &FunctionGraph, node: NodeId) -> bool {
    match graph.value_type(node) {
        ValueType::Tuple(outputs) => outputs.get(2).is_some_and(|ty| scalar_type(graph, ty)),
        _ => false,
    }
}

fn scalar(graph: &FunctionGraph, node: NodeId) -> bool {
    scalar_type(graph, graph.value_type(graph.resolve(node)))
}

fn scalar_type(graph: &FunctionGraph, ty: &ValueType) -> bool {
    let ValueType::Rust(ty) = ty else {
        return false;
    };
    matches!(graph.oracle.shape(*ty), TypeShape::Bool | TypeShape::Char)
        || matches!(graph.oracle.shape(*ty), TypeShape::Int { width, .. } if width.bits(64) <= 64)
}
