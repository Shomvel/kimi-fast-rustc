//! A chain with one observable hit destination can use a range or a bitmap.
//! The miss stays in place, including when it starts another dispatch.
use super::{MAX_WRAPPERS, Membership, Plan as Chain, jump_target};
use crate::analysis::uses::{UseKind, Uses};
use crate::transforms::utils::cfg::CfgEditor;
use crate::{ComparePredicate as P, FunctionGraph, NodeId, Opcode, ValueType};
use kfrc_body::{IntegerWidth, ScalarValidRange, SizeAlign, TyId, TypeOracle, TypeShape};

pub(super) const MIN_CASES: usize = 3;

#[derive(Clone, Copy)]
pub(super) enum Test {
    Range {
        low: u128,
        span: u128,
    },
    Bits {
        low: u128,
        limit: Option<u128>,
        mask: u64,
        width: u8,
    },
}

#[derive(Clone, Copy)]
enum Bounds {
    Guarded(u128),
    Typed(ScalarValidRange),
}

fn bitmap_width(mask: u64) -> u8 {
    if u32::try_from(mask).is_ok() { 32 } else { 64 }
}

fn range_fits_index(range: ScalarValidRange, low: u128, width: u8) -> bool {
    let distance = |bits: u128| bits.wrapping_sub(low) & range.mask();
    let start = distance(range.start());
    let end = distance(range.end());
    // Subtraction rotates the interval in the input width. When the rotated
    // range wraps, some index reaches the maximum (at least 255), regardless
    // of whether both endpoints appear small in another representation.
    !range.is_full() && start <= end && end < u128::from(width)
}

impl Test {
    fn select(cases: &[super::Case], bounds: Bounds) -> Option<Self> {
        let values = || cases.iter().map(|case| case.test.bits);
        let (low, high) = (values().min()?, values().max()?);
        let span = high - low;
        let count = cases.len() as u128;
        if let Bounds::Typed(range) = bounds {
            if span + 1 == count {
                // Keep this experiment focused on non-contiguous tests.
                return None;
            }
            for low in [0, low, range.start()] {
                let indices = values().map(|bits| bits.wrapping_sub(low) & range.mask());
                if indices.clone().any(|index| index >= 64) {
                    continue;
                }
                let mask = indices.fold(0, |mask, index| mask | (1u64 << index));
                for width in [bitmap_width(mask), 64] {
                    // A wide test or a bias may cost an extra instruction.
                    if cases.len() >= MIN_CASES + usize::from(width > 32 || low != 0)
                        && range_fits_index(range, low, width)
                    {
                        return Some(Self::Bits {
                            low,
                            limit: None,
                            mask,
                            width,
                        });
                    }
                }
            }
            return None;
        }
        let Bounds::Guarded(domain) = bounds else {
            unreachable!()
        };
        if span + 1 == count {
            return Some(Self::Range { low, span });
        }
        if low == 0 && high == domain {
            // An interval crossing zero has one gap in unsigned sort order.
            let mut sorted: Vec<_> = values().collect();
            sorted.sort_unstable();
            if let Some(gap) = sorted.windows(2).find(|pair| pair[1] - pair[0] != 1) {
                let span = domain + 1 - (gap[1] - gap[0]);
                if span + 1 == count {
                    return Some(Self::Range { low: gap[1], span });
                }
            }
        }
        // A guard and bitmap replace at least three compares. Require more
        // comparisons to pay for a bias subtraction or a wide mask constant.
        let (low, span) = match (high, span, cases.len()) {
            (0..=31, _, _) | (0..=63, _, 4..) => (0, high),
            (_, 0..=31, 4..) | (_, 0..=63, 5..) => (low, span),
            _ => return None,
        };
        let mask = values().fold(0, |mask, bits| mask | (1u64 << (bits - low)));
        Some(Self::Bits {
            low,
            limit: Some(span),
            mask,
            width: bitmap_width(mask),
        })
    }
}

/// Read only a bound of this exact typed load. Rewrites that forward the
/// observed value, clone the load or combine reads don't acquire this fact.
fn certified_load(graph: &FunctionGraph, value: NodeId, width: u8) -> Option<ScalarValidRange> {
    let ranges = graph.enum_tag_load_ranges.as_deref()?;
    if graph.op(value) != Opcode::Projection || graph.payload(value) != 2 {
        return None;
    }
    let load = graph.input(value, 0);
    if graph.op(load) != Opcode::Load {
        return None;
    }
    ranges
        .get(&load)
        .copied()
        .filter(|range| range.bits() == width)
}

struct Path {
    region: NodeId,
    slot: usize,
    nodes: Vec<NodeId>,
}

impl Path {
    fn collect(
        graph: &FunctionGraph,
        uses: &Uses,
        mut control: NodeId,
        visits: &mut usize,
    ) -> Option<Self> {
        let mut nodes = Vec::new();
        for _ in 0..=MAX_WRAPPERS {
            *visits += 1;
            let [usage] = uses.of(control) else {
                return None;
            };
            if usage.kind() != UseKind::Control {
                return None;
            }
            let jump = usage.user;
            let (region, slot) = jump_target(graph, uses, jump)?;
            nodes.extend([control, jump]);
            if graph.inputs(region).len() > 1 {
                return Some(Self {
                    region,
                    slot,
                    nodes,
                });
            }
            control = region;
        }
        None
    }
}

pub(super) struct Plan {
    pub(super) test: Test,
    region: NodeId,
    redundant: Vec<Path>,
}

impl Plan {
    pub(super) fn collect(
        graph: &FunctionGraph,
        uses: &Uses,
        chain: &Chain,
        policy: Membership,
        visits: &mut usize,
    ) -> Option<Self> {
        if policy == Membership::Off || chain.cases.len() < MIN_CASES {
            return None;
        }
        let ty = graph.rust_type(chain.value)?;
        let TypeShape::Int { width, .. } = graph.oracle.shape(ty) else {
            return None;
        };
        if graph.oracle.scalar_validity(ty).is_some() {
            return None;
        }
        let width = width.bits(64);
        let bounds = if policy == Membership::Typed {
            Bounds::Typed(certified_load(graph, chain.value, width)?)
        } else {
            Bounds::Guarded((1u128 << width) - 1)
        };
        let first = Path::collect(graph, uses, chain.cases[0].test.yes, visits)?;
        let mut phis = Vec::new();
        for usage in uses.of(first.region) {
            if usage.kind() == UseKind::Control {
                continue;
            }
            let phi = usage.user;
            if usage.kind() != UseKind::Operand(0)
                || graph.op(phi) != Opcode::Phi
                || graph.inputs(phi).len() != graph.inputs(first.region).len() + 1
            {
                return None;
            }
            phis.push(&graph.inputs(phi)[1..]);
        }
        let mut redundant = Vec::new();
        for case in &chain.cases[1..] {
            let Some(path) = Path::collect(graph, uses, case.test.yes, visits) else {
                break;
            };
            if path.region != first.region
                || path.slot == first.slot
                || phis
                    .iter()
                    .any(|inputs| inputs[path.slot] != inputs[first.slot])
            {
                break;
            }
            // The exclusive uses along both paths imply distinct predecessor
            // jumps. The comparison includes Effect Phis as well as values.
            redundant.push(path);
        }
        let (len, test) = (MIN_CASES..=1 + redundant.len())
            .rev()
            .find_map(|len| Test::select(&chain.cases[..len], bounds).map(|test| (len, test)))?;
        redundant.truncate(len - 1);
        Some(Self {
            test,
            region: first.region,
            redundant,
        })
    }

    pub(super) fn len(&self) -> usize {
        self.redundant.len() + 1
    }

    pub(super) fn apply(self, graph: &mut FunctionGraph, chain: Chain) {
        let ty = graph.rust_type(chain.value).unwrap();
        let boolean = graph
            .rust_type(graph.input(chain.cases[0].branch, 0))
            .unwrap();
        let (low, limit) = match self.test {
            Test::Range { low, span } => (low, Some(span)),
            Test::Bits { low, limit, .. } => (low, limit),
        };
        // Keep the subtraction and guard in the original width. In particular,
        // a larger word must not alias a set bit when narrowed for the shift.
        let offset = if low == 0 {
            chain.value
        } else {
            let low = graph.add_constant(low, ty);
            pure(graph, Opcode::Sub, &[chain.value, low], 0, ty)
        };
        let check = limit.map(|limit| {
            let limit = graph.add_constant(limit, ty);
            let in_range = pure(
                graph,
                Opcode::Compare,
                &[offset, limit],
                P::UnsignedLessEqual as u32,
                boolean,
            );
            branch(graph, chain.control, in_range)
        });
        let [hit, miss] = match self.test {
            Test::Range { .. } => check.unwrap(),
            Test::Bits { mask, width, .. } => {
                let (width, bytes) = if width == 32 {
                    (IntegerWidth::W32, 4)
                } else {
                    (IntegerWidth::W64, 8)
                };
                let word = graph.oracle.intern(
                    TypeShape::Int {
                        width,
                        signed: false,
                    },
                    Some(SizeAlign {
                        size: bytes,
                        align: bytes,
                    }),
                );
                // The guard or typed range establishes offset <= 63, so
                // extension of a signed narrow operand and truncation of a
                // wide operand both preserve its value.
                let index = if word == ty {
                    offset
                } else {
                    pure(graph, Opcode::IntCast, &[offset], 0, word)
                };
                let one = graph.add_constant(1, word);
                let mask = graph.add_constant(u128::from(mask), word);
                let zero = graph.add_constant(0, word);
                let bit = pure(graph, Opcode::ShiftLeft, &[one, index], 0, word);
                let selected = pure(graph, Opcode::BitAnd, &[mask, bit], 0, word);
                let contains = pure(
                    graph,
                    Opcode::Compare,
                    &[selected, zero],
                    P::NotEqual as u32,
                    boolean,
                );
                if let Some([inside, outside]) = check {
                    let [hit, unset] = branch(graph, inside, contains);
                    let outside = graph.add_node(Opcode::Jump, outside, &[], 0, ValueType::Control);
                    let unset = graph.add_node(Opcode::Jump, unset, &[], 0, ValueType::Control);
                    let miss = graph.add_node(
                        Opcode::Region,
                        NodeId::NONE,
                        &[outside, unset],
                        0,
                        ValueType::Control,
                    );
                    [hit, miss]
                } else {
                    branch(graph, chain.control, contains)
                }
            }
        };
        graph.replace(chain.cases[0].test.yes, hit);
        graph.replace(chain.cases.last().unwrap().test.no, miss);
        // Find predecessor slots by jump identity. An earlier disjoint plan
        // can already have removed other predecessors of this same region.
        let keep: Vec<_> = graph
            .inputs(self.region)
            .iter()
            .map(|jump| {
                !self
                    .redundant
                    .iter()
                    .any(|path| path.nodes.last() == Some(jump))
            })
            .collect();
        CfgEditor::new(graph)
            .retain_predecessors(self.region, &keep)
            .expect("membership merge proved its predecessor and Phi slots");
        for path in self.redundant {
            for node in path.nodes {
                graph.remove_node(node);
            }
        }
        chain.retire_chain(graph);
    }
}

fn pure(
    graph: &mut FunctionGraph,
    op: Opcode,
    inputs: &[NodeId],
    payload: u32,
    ty: TyId,
) -> NodeId {
    graph.add_node(op, NodeId::NONE, inputs, payload, ValueType::Rust(ty))
}

fn branch(graph: &mut FunctionGraph, control: NodeId, condition: NodeId) -> [NodeId; 2] {
    let node = graph.add_node(
        Opcode::If,
        control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    [0, 1].map(|slot| {
        graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[node],
            slot,
            ValueType::Control,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modular_range_proof_agrees_with_every_byte() {
        for (start, end) in [
            (0, 16),
            (4, 63),
            (0, 64),
            (245, 5),
            (245, 244),
            (255, 0),
            (255, 255),
        ] {
            let range = ScalarValidRange::new(8, start, end).unwrap();
            for low in 0..=255 {
                for width in [32, 64] {
                    let expected = (0u128..=255)
                        .filter(|&bits| range.contains(bits))
                        .all(|bits| bits.wrapping_sub(low) & 255 < u128::from(width));
                    assert_eq!(
                        range_fits_index(range, low, width),
                        expected,
                        "{start}..{end}, bias {low}, word {width}"
                    );
                }
            }
        }
    }

    #[test]
    fn fact_survives_renumbering_but_cannot_follow_an_alias() {
        use crate::GraphLevel;
        use kfrc_body::{FunctionAbi, PassMode, RecordedTypes};

        let mut graph = FunctionGraph::new(
            "tags",
            FunctionAbi {
                args: vec![],
                ret: PassMode::Ignore,
                c_variadic: false,
            },
            RecordedTypes::new(),
        );
        graph.level = GraphLevel::Layout;
        let ty = graph.oracle.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            None,
        );
        graph.add_constant(0, ty); // Retired by compaction, moving every subsequent id.
        let load = graph.add_node(Opcode::Load, NodeId::NONE, &[], 0, ValueType::Control);
        let other = graph.add_node(Opcode::Load, NodeId::NONE, &[], 0, ValueType::Control);
        let result = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[load],
            2,
            ValueType::Rust(ty),
        );
        let ret = graph.add_node(
            Opcode::Return,
            NodeId::NONE,
            &[result],
            0,
            ValueType::Control,
        );
        let range = ScalarValidRange::new(8, 0, 16).unwrap();
        graph
            .enum_tag_load_ranges
            .get_or_insert_with(Default::default)
            .insert(load, range);
        graph.compact();
        let result = graph.input(NodeId(ret.0 - 1), 0);
        assert_eq!(certified_load(&graph, result, 8), Some(range));
        let load = graph.input(result, 0);
        graph.replace(load, NodeId(other.0 - 1));
        graph.compact();
        let ret = graph
            .ids()
            .find(|&node| graph.op(node) == Opcode::Return)
            .unwrap();
        assert_eq!(certified_load(&graph, graph.input(ret, 0), 8), None);
    }
}
