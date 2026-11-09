//! Ranges of definitely initialized values in S. Nominal validity alone is
//! never enough: raw loads, ConstRepr, Undefined and flagged arithmetic do not
//! enter this analysis. No path fact is cached across dynamic loop instances.
use std::collections::{HashMap, HashSet};

use crate::{ComparePredicate, FunctionGraph, GraphLevel, NodeId, Opcode};
use kfrc_body::{ScalarValidRange, TyId, TypeOracle, TypeShape};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValueRange {
    pub bits: u8,
    pub min: u64,
    pub max: u64,
}
impl ValueRange {
    fn full(bits: u8) -> Self {
        Self {
            bits,
            min: 0,
            max: u64::MAX >> (64 - bits),
        }
    }
    fn validity(range: ScalarValidRange) -> Option<Self> {
        let bits = range.bits();
        if bits > 64 {
            return None;
        }
        Some(if range.start() <= range.end() {
            Self {
                bits,
                min: range.start() as u64,
                max: range.end() as u64,
            }
        } else {
            Self::full(bits)
        })
    }
    fn union(self, other: Self) -> Option<Self> {
        (self.bits == other.bits).then_some(Self {
            bits: self.bits,
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        })
    }
    fn intersect(self, other: Self) -> Option<Self> {
        if self.bits != other.bits {
            return None;
        }
        let min = self.min.max(other.min);
        let max = self.max.min(other.max);
        (min <= max).then_some(Self {
            bits: self.bits,
            min,
            max,
        })
    }
    fn signed_bounds(self) -> (i128, i128) {
        let sign = 1u64 << (self.bits - 1);
        if self.min < sign && self.max >= sign {
            (-(1i128 << (self.bits - 1)), (1i128 << (self.bits - 1)) - 1)
        } else if self.min >= sign {
            (
                i128::from(self.min) - (1i128 << self.bits),
                i128::from(self.max) - (1i128 << self.bits),
            )
        } else {
            (i128::from(self.min), i128::from(self.max))
        }
    }
}

pub struct RangeFacts<'a> {
    graph: &'a FunctionGraph,
    memo: HashMap<NodeId, Option<ValueRange>>,
    active: HashSet<NodeId>,
    validated: HashMap<NodeId, bool>,
    pub queries: usize,
    pub node_visits: usize,
}
impl<'a> RangeFacts<'a> {
    pub fn new(graph: &'a FunctionGraph) -> Self {
        Self {
            graph,
            memo: HashMap::new(),
            active: HashSet::new(),
            validated: HashMap::new(),
            queries: 0,
            node_visits: 0,
        }
    }
    pub fn range_of(&mut self, node: NodeId) -> Option<ValueRange> {
        self.queries += 1;
        if self.graph.level != GraphLevel::Specialized {
            return None;
        }
        self.range(node, &mut 256)
    }
    fn range(&mut self, node: NodeId, budget: &mut usize) -> Option<ValueRange> {
        let node = self.graph.resolve(node);
        if let Some(range) = self.memo.get(&node) {
            return *range;
        }
        if *budget == 0 || !self.active.insert(node) {
            return None;
        }
        *budget -= 1;
        self.node_visits += 1;
        let range = self.compute(node, budget);
        self.active.remove(&node);
        self.memo.insert(node, range);
        range
    }
    fn type_range(&self, ty: TyId) -> Option<ValueRange> {
        if self.graph.oracle.semantic().is_union(ty) {
            return None;
        }
        if let Some(range) = self.graph.oracle.scalar_valid_range(ty) {
            let mut range = ValueRange::validity(range)?;
            if matches!(self.graph.oracle.shape(ty), TypeShape::Bool) {
                range.bits = 1;
            }
            return Some(range);
        }
        let (bits, _) = scalar_kind(self.graph, ty)?;
        Some(match self.graph.oracle.shape(ty) {
            TypeShape::Bool => ValueRange {
                bits,
                min: 0,
                max: 1,
            },
            TypeShape::Char => ValueRange {
                bits,
                min: 0,
                max: 0x10ffff,
            },
            _ => ValueRange::full(bits),
        })
    }
    /// Full type validity is established by these boundary operations on every
    /// execution, independently of whether the static NodeId is in a loop.
    fn typed(&mut self, node: NodeId, budget: &mut usize) -> bool {
        let node = self.graph.resolve(node);
        if let Some(known) = self.validated.get(&node) {
            return *known;
        }
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        self.node_visits += 1;
        let g = self.graph;
        let known = match g.op(node) {
            Opcode::Projection if g.payload(node) >= 2 => matches!(
                g.op(g.input(node, 0)),
                Opcode::Start | Opcode::Call | Opcode::CallIndirect | Opcode::TransmuteValue
            ),
            Opcode::Field => self.typed(g.input(node, 0), budget),
            _ => false,
        };
        self.validated.insert(node, known);
        known
    }
    fn compute(&mut self, node: NodeId, budget: &mut usize) -> Option<ValueRange> {
        let g = self.graph;
        let ty = g.rust_type(node)?;
        match g.op(node) {
            Opcode::BitCount => {
                let source = self.range(g.input(node, 0), budget)?;
                let nonzero = g.payload(node) & crate::IntegerIntrinsicPayload::ZERO_POISON != 0;
                if nonzero && source.min == 0 {
                    return None;
                }
                Some(ValueRange {
                    bits: 32,
                    min: 0,
                    max: u64::from(source.bits) - u64::from(nonzero),
                })
            }
            Opcode::Const => {
                let (bits, _) = scalar_kind(g, ty)?;
                let value = g.constant_value(node);
                if value > u128::from(ValueRange::full(bits).max) {
                    return None;
                }
                if let Some(valid) = g.oracle.scalar_valid_range(ty)
                    && !valid.contains(value)
                {
                    return None;
                }
                Some(ValueRange {
                    bits,
                    min: value as u64,
                    max: value as u64,
                })
            }
            Opcode::Projection if self.typed(node, budget) => {
                let source = g.input(node, 0);
                let range = self.type_range(ty)?;
                if g.op(source) == Opcode::TransmuteValue {
                    // A transparent NonZero -> integer transmute keeps the
                    // source representation restriction after nominal erasure.
                    self.range(g.input(source, 1), budget)
                        .and_then(|source| range.intersect(source))
                        .or(Some(range))
                } else {
                    Some(range)
                }
            }
            Opcode::Field => {
                let source = g.input(node, 0);
                let index = g.payload(node) as usize;
                if g.op(source) == Opcode::Aggregate {
                    return self.range(*g.inputs(source).get(index)?, budget);
                }
                if !self.typed(source, budget) {
                    return None;
                }
                let own = self.type_range(ty)?;
                let source_ty = g.rust_type(source)?;
                // A field can have weaker validity than its enclosing newtype.
                // Only exact scalar-lane boundaries retain the parent fact.
                let field = g.oracle.aggregates.get(&source_ty)?.fields.get(index)?;
                let size = g.oracle.size_align(ty)?.size;
                let inherited = g.oracle.scalar_validity(source_ty).and_then(|validity| {
                    validity.lanes().find_map(|(offset, range)| {
                        (field.offset == offset && u64::from(range.bits()).div_ceil(8) == size)
                            .then(|| ValueRange::validity(range))
                            .flatten()
                    })
                });
                inherited
                    .and_then(|range| own.intersect(range))
                    .or(Some(own))
            }
            Opcode::IntCast => {
                let source = g.input(node, 0);
                let range = self.range(source, budget)?;
                let (_, signed) = scalar_kind(g, g.rust_type(source)?)?;
                let (bits, _) = scalar_kind(g, ty)?;
                Some(cast(range, bits, signed))
            }
            Opcode::PointerCast => {
                let range = self.range(g.input(node, 0), budget)?;
                let (bits, _) = scalar_kind(g, ty)?;
                (range.bits == bits).then_some(range)
            }
            Opcode::BitAnd if g.payload(node) == 0 => {
                let left = self.range(g.input(node, 0), budget)?;
                let right = self.range(g.input(node, 1), budget)?;
                (left.bits == right.bits).then_some(ValueRange {
                    bits: left.bits,
                    min: 0,
                    max: left.max.min(right.max),
                })
            }
            Opcode::ShiftRightLogical if g.payload(node) == 0 => {
                let value = self.range(g.input(node, 0), budget)?;
                let shift = self.range(g.input(node, 1), budget)?;
                if shift.min != shift.max {
                    return None;
                }
                let amount = shift.min % u64::from(value.bits);
                Some(ValueRange {
                    bits: value.bits,
                    min: value.min >> amount,
                    max: value.max >> amount,
                })
            }
            Opcode::VariantOf if self.typed(g.input(node, 0), budget) => {
                let source_ty = g.rust_type(g.input(node, 0))?;
                let count = g.oracle.semantic().variant_count(source_ty)?;
                if count == 0 {
                    return None;
                }
                let (bits, _) = scalar_kind(g, ty)?;
                Some(ValueRange {
                    bits,
                    min: 0,
                    max: count as u64 - 1,
                })
            }
            Opcode::Phi => {
                // A recursion cycle is unknown: no self-justifying loop range.
                let mut inputs = g.inputs(node)[1..].iter();
                let mut range = self.range(*inputs.next()?, budget)?;
                for input in inputs {
                    range = range.union(self.range(*input, budget)?)?;
                }
                Some(range)
            }
            _ => None,
        }
    }

    pub fn compare(&mut self, node: NodeId) -> Option<bool> {
        if self.graph.op(node) != Opcode::Compare {
            return None;
        }
        let left = self.range_of(self.graph.input(node, 0))?;
        let right = self.range_of(self.graph.input(node, 1))?;
        if left.bits != right.bits {
            return None;
        }
        use ComparePredicate::*;
        let predicate = ComparePredicate::from_payload(self.graph.payload(node));
        let bounds = match predicate {
            SignedLess | SignedLessEqual | SignedGreater | SignedGreaterEqual => {
                (left.signed_bounds(), right.signed_bounds())
            }
            _ => (
                (i128::from(left.min), i128::from(left.max)),
                (i128::from(right.min), i128::from(right.max)),
            ),
        };
        let ((a, b), (c, d)) = bounds;
        match predicate {
            Equal | NotEqual if b < c || d < a => Some(predicate == NotEqual),
            Equal | NotEqual if a == b && c == d && a == c => Some(predicate == Equal),
            SignedLess | UnsignedLess if b < c => Some(true),
            SignedLess | UnsignedLess if a >= d => Some(false),
            SignedLessEqual | UnsignedLessEqual if b <= c => Some(true),
            SignedLessEqual | UnsignedLessEqual if a > d => Some(false),
            SignedGreater | UnsignedGreater if a > d => Some(true),
            SignedGreater | UnsignedGreater if b <= c => Some(false),
            SignedGreaterEqual | UnsignedGreaterEqual if a >= d => Some(true),
            SignedGreaterEqual | UnsignedGreaterEqual if b < c => Some(false),
            _ => None,
        }
    }
}

fn scalar_kind(g: &FunctionGraph, ty: TyId) -> Option<(u8, bool)> {
    match g.oracle.shape(ty) {
        TypeShape::Bool => Some((1, false)),
        TypeShape::Char => Some((32, false)),
        TypeShape::Int { width, signed } if width.bits(64) <= 64 => Some((width.bits(64), *signed)),
        TypeShape::RawPointer { .. } | TypeShape::Reference { .. }
            if !g.oracle.semantic().is_product(ty) =>
        {
            Some((64, false))
        }
        _ => None,
    }
}

fn cast(range: ValueRange, bits: u8, signed: bool) -> ValueRange {
    if bits == range.bits {
        return range;
    }
    let full = ValueRange::full(bits);
    if bits < range.bits {
        if range.min >> bits == range.max >> bits {
            return ValueRange {
                bits,
                min: range.min & full.max,
                max: range.max & full.max,
            };
        }
        return full;
    }
    if !signed || range.max < (1u64 << (range.bits - 1)) {
        return ValueRange { bits, ..range };
    }
    if range.min >= (1u64 << (range.bits - 1)) {
        let extension = full.max ^ ValueRange::full(range.bits).max;
        return ValueRange {
            bits,
            min: range.min | extension,
            max: range.max | extension,
        };
    }
    full
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_interval_cast_bounds_cover_each_concrete_cast() {
        // Includes intervals crossing signed zero and truncation boundaries.
        for min in (0..=255).step_by(7) {
            for max in (min..=255).step_by(11) {
                let input = ValueRange { bits: 8, min, max };
                for bits in [4, 8, 16, 64] {
                    for signed in [false, true] {
                        let range = cast(input, bits, signed);
                        for value in min..=max {
                            let extended = if signed {
                                (value as u8 as i8 as i64) as u64
                            } else {
                                value
                            };
                            let expected = extended & ValueRange::full(bits).max;
                            assert!(
                                range.min <= expected && expected <= range.max,
                                "{input:?}, bits={bits}, signed={signed}, value={value}: {range:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}
