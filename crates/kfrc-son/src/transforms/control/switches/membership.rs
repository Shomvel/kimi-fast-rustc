//! A chain with one observable hit destination can use a range or a bitmap.
//! The miss stays in place, including when it starts another dispatch.
use super::{MAX_WRAPPERS, Plan as Chain, jump_target};
use crate::analysis::uses::{UseKind, Uses};
use crate::transforms::utils::cfg::CfgEditor;
use crate::{ComparePredicate as P, FunctionGraph, NodeId, Opcode, ValueType};
use kfrc_body::{IntegerWidth, SizeAlign, TyId, TypeOracle, TypeShape};

pub(super) const MIN_CASES: usize = 3;

#[derive(Clone, Copy)]
pub(super) enum Test {
    Range { low: u128, span: u128 },
    Bits { low: u128, span: u128, mask: u64 },
}

impl Test {
    fn select(cases: &[super::Case], domain: u128) -> Option<Self> {
        let values = || cases.iter().map(|case| case.test.bits);
        let (low, high) = (values().min()?, values().max()?);
        let span = high - low;
        let count = cases.len() as u128;
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
        Some(Self::Bits { low, span, mask })
    }
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
        visits: &mut usize,
    ) -> Option<Self> {
        if chain.cases.len() < MIN_CASES {
            return None;
        }
        let ty = graph.rust_type(chain.value)?;
        let TypeShape::Int { width, .. } = graph.oracle.shape(ty) else {
            return None;
        };
        if graph.oracle.scalar_validity(ty).is_some() {
            return None;
        }
        let domain = (1u128 << width.bits(64)) - 1;
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
            .find_map(|len| Test::select(&chain.cases[..len], domain).map(|test| (len, test)))?;
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
        let (low, span) = match self.test {
            Test::Range { low, span } | Test::Bits { low, span, .. } => (low, span),
        };
        // Keep the subtraction and guard in the original width. In particular,
        // a larger word must not alias a set bit when narrowed for the shift.
        let offset = if low == 0 {
            chain.value
        } else {
            let low = graph.add_constant(low, ty);
            pure(graph, Opcode::Sub, &[chain.value, low], 0, ty)
        };
        let limit = graph.add_constant(span, ty);
        let in_range = pure(
            graph,
            Opcode::Compare,
            &[offset, limit],
            P::UnsignedLessEqual as u32,
            boolean,
        );
        let [inside, outside] = branch(graph, chain.control, in_range);
        let (hit, miss) = match self.test {
            Test::Range { .. } => (inside, outside),
            Test::Bits { mask, .. } => {
                let (width, bytes) = if u32::try_from(mask).is_ok() {
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
                // Here the guard establishes offset <= 63, so extension of
                // a signed narrow operand and truncation of a wide operand
                // both preserve its value. The shift alone would wrap indices.
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
                (hit, miss)
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
