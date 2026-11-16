//! Total operand evaluation, distinct from proving that a value is defined.
//! Poison-producing scalar operations are allowed: storing poison observes
//! nothing. Representation decoding and unknown execution/encoding are not.
use crate::{FunctionGraph, NodeId, Opcode, ValueType};
use kfrc_body::{TyId, TypeOracle, TypeShape};

#[derive(Default)]
pub(crate) struct EvaluationStats {
    pub node_visits: usize,
    pub budget_bailouts: usize,
}

pub(crate) struct TotalEvaluation {
    values: Vec<Option<bool>>,
    check_phi_inputs: bool,
}

impl TotalEvaluation {
    pub(crate) fn new(graph: &FunctionGraph) -> Self {
        Self {
            values: vec![None; graph.len()],
            check_phi_inputs: false,
        }
    }

    /// Use when later cleanup may retire a Phi whose last consumer is erased.
    pub(crate) fn checking_phi_inputs(graph: &FunctionGraph) -> Self {
        Self {
            check_phi_inputs: true,
            ..Self::new(graph)
        }
    }

    /// Only complete scalar/Unit encodings enter this store proof. Enum writes
    /// can fail while selecting an uninhabited variant, or leave bytes of the
    /// previous payload untouched. Aggregate encoding needs its own footprint
    /// proof; same type alone is not a complete byte-overwrite guarantee.
    pub(crate) fn operands_and_encoding(
        &mut self,
        graph: &FunctionGraph,
        store: NodeId,
        fuel: &mut usize,
        stats: &mut EvaluationStats,
    ) -> bool {
        let value = graph.resolve(graph.input(store, 2));
        complete_scalar_encoding(graph, value)
            && self.total_value(graph, graph.input(store, 1), fuel, stats, &mut 64, 0)
            && self.total_value(graph, value, fuel, stats, &mut 64, 0)
    }

    /// A fixed result is available only if its producer's execution survives.
    /// Call before querying an immutable graph snapshot.
    pub(crate) fn exclude_producer(&mut self, graph: &FunctionGraph, producer: NodeId) {
        if let Some(result) = graph.find_projection(producer, 2) {
            self.values[result.index()] = Some(false);
        }
    }

    pub(crate) fn is_total(
        &mut self,
        graph: &FunctionGraph,
        node: NodeId,
        fuel: &mut usize,
        stats: &mut EvaluationStats,
    ) -> bool {
        self.total_value(graph, node, fuel, stats, &mut 64, 0)
    }

    fn total_value(
        &mut self,
        graph: &FunctionGraph,
        node: NodeId,
        fuel: &mut usize,
        stats: &mut EvaluationStats,
        remaining: &mut usize,
        depth: usize,
    ) -> bool {
        let node = graph.resolve(node);
        if let Some(result) = self.values[node.index()] {
            return result;
        }
        if *remaining == 0 || *fuel == 0 || depth == 32 {
            stats.budget_bailouts += 1;
            return false;
        }
        *remaining -= 1;
        *fuel -= 1;
        stats.node_visits += 1;
        // A cycle cannot justify itself. Failed or budget-limited queries stay
        // conservative for this immutable graph snapshot.
        self.values[node.index()] = Some(false);
        if graph.rust_type(node).is_none() {
            return false;
        }
        let op = graph.op(node);
        let anchored = matches!(
            op,
            Opcode::DivSigned | Opcode::DivUnsigned | Opcode::RemSigned | Opcode::RemUnsigned
        );
        let control = graph.resolve(graph.control(node));
        if !anchored && !control.is_none() {
            return false;
        }
        let result = match op {
            Opcode::Const | Opcode::Undefined | Opcode::LocalSlot | Opcode::SymbolAddress => {
                complete_scalar_encoding(graph, node)
            }
            // A verified graph places these definitions before each use. S
            // evaluates every Phi on entry and anchored Div/Rem at its anchor,
            // even when its value is unused. The caller retains those execution sites.
            Opcode::Phi => {
                let region = graph.input(node, 0);
                complete_scalar_encoding(graph, node)
                    && graph.op(region) == Opcode::Region
                    && !graph.inputs(region).is_empty()
                    && graph.inputs(node).len() == graph.inputs(region).len() + 1
                    && (!self.check_phi_inputs
                        || graph.inputs(node)[1..].iter().all(|&input| {
                            self.total_value(graph, input, fuel, stats, remaining, depth + 1)
                        }))
            }
            _ if anchored => {
                complete_scalar_encoding(graph, node)
                    && !control.is_none()
                    && graph.value_type(control) == &ValueType::Control
            }
            Opcode::Projection => available_projection(graph, node),
            // Decoding ConstRepr can observe invalid bool/char/nominal lanes,
            // tags or uninhabited encodings. It is not merely poison creation.
            Opcode::ConstRepr => false,
            Opcode::Repeat => {
                let ty = graph.rust_type(node).unwrap();
                matches!(graph.oracle.shape(ty), TypeShape::Array { length: 0, .. })
                    || self.total_value(
                        graph,
                        graph.input(node, 0),
                        fuel,
                        stats,
                        remaining,
                        depth + 1,
                    )
            }
            Opcode::VariantField
                if graph.inputs(node).len() != 1 || !complete_scalar_encoding(graph, node) =>
            {
                false
            }
            Opcode::Aggregate
            | Opcode::MakeVariant
            | Opcode::WidePtr
            | Opcode::Field
            | Opcode::InsertField
            | Opcode::VariantField
            | Opcode::VariantOf
            | Opcode::Discriminant
            | Opcode::PointerCast
            | Opcode::FieldAddr
            | Opcode::VariantFieldAddr
            | Opcode::IndexAddr
            | Opcode::Add
            | Opcode::Sub
            | Opcode::Mul
            | Opcode::BitAnd
            | Opcode::BitOr
            | Opcode::BitXor
            | Opcode::ShiftLeft
            | Opcode::ShiftRightArithmetic
            | Opcode::ShiftRightLogical
            | Opcode::Negate
            | Opcode::BitNot
            | Opcode::Compare
            | Opcode::IntCast
            | Opcode::PtrOffset
            | Opcode::MulOverflow
            | Opcode::BitCount
            | Opcode::ByteSwap
            | Opcode::BitReverse
            | Opcode::FunnelShift
            | Opcode::SaturatingAdd
            | Opcode::SaturatingSub => graph
                .inputs(node)
                .iter()
                .all(|&input| self.total_value(graph, input, fuel, stats, remaining, depth + 1)),
            _ => false,
        };
        self.values[node.index()] = Some(result);
        result
    }
}

fn complete_scalar_encoding(graph: &FunctionGraph, value: NodeId) -> bool {
    graph
        .rust_type(value)
        .is_some_and(|ty| complete_scalar_layout(graph, ty))
}

pub(crate) fn complete_scalar_layout(graph: &FunctionGraph, ty: TyId) -> bool {
    let semantic = graph.oracle.semantic();
    if semantic.is_enum(ty)
        || semantic.is_product(ty) && !matches!(semantic.shape(ty), TypeShape::Unit)
        || semantic.maybe_uninit_inner(ty).is_some()
    {
        return false;
    }
    let size = match semantic.shape(ty) {
        TypeShape::Unit => 0,
        TypeShape::Bool => 1,
        TypeShape::Char => 4,
        TypeShape::Int { width, .. } if width.bits(64) <= 64 => u64::from(width.bits(64) / 8),
        TypeShape::Reference { .. }
        | TypeShape::RawPointer { .. }
        | TypeShape::PlaceAddress { .. } => 8,
        _ => return false,
    };
    graph
        .oracle
        .size_align(ty)
        .is_some_and(|layout| layout.size == size && layout.align.is_power_of_two())
}

fn available_projection(graph: &FunctionGraph, node: NodeId) -> bool {
    // Keep availability separate from a claim about arbitrary aggregate
    // runtime shapes. This proof consumes only scalar/Unit results; compound
    // results need a fuller producer-shape analysis.
    if !complete_scalar_encoding(graph, node) {
        return false;
    }
    let source = graph.resolve(graph.input(node, 0));
    let index = graph.payload(node) as usize;
    let ValueType::Tuple(results) = graph.value_type(source) else {
        return false;
    };
    if results.get(index) != Some(graph.value_type(node)) {
        return false;
    }
    if source == graph.start {
        // Start validates all arguments, before the candidate store executes.
        return index >= 2;
    }
    index == 2
        && matches!(
            graph.op(source),
            Opcode::Call
                | Opcode::CallIndirect
                | Opcode::LoadValue
                | Opcode::Opaque
                | Opcode::TransmuteValue
        )
        && !graph.control(source).is_none()
        // The interpreter records precisely the canonical result projection.
        // An extra tuple projection is not an already-executed value there.
        && graph.find_projection(source, 2) == Some(node)
}
