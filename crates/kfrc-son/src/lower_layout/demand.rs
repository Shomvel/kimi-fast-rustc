//! Rust value-field demands, computed before changing the S graph.
//! A missing leaf is not poison: it has no execution consumer and no L node.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Key {
    Field(u32),
    VariantField(u32, u32),
    Tag,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum Demand {
    #[default]
    None,
    All,
    Fields(BTreeMap<Key, Demand>),
}
impl Demand {
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
    pub fn child(&self, key: Key) -> Self {
        match self {
            Self::All => Self::All,
            Self::Fields(fields) => fields.get(&key).cloned().unwrap_or_default(),
            Self::None => Self::None,
        }
    }
    fn path(key: Key, child: Self) -> Self {
        if child.is_none() {
            Self::None
        } else {
            Self::Fields(BTreeMap::from([(key, child)]))
        }
    }
    fn merge(&mut self, other: &Self) -> bool {
        if other.is_none() || matches!(self, Self::All) {
            return false;
        }
        if matches!(other, Self::All) || self.is_none() {
            *self = other.clone();
            return true;
        }
        let (Self::Fields(left), Self::Fields(right)) = (self, other) else {
            unreachable!()
        };
        let mut changed = false;
        for (&key, demand) in right {
            changed |= left.entry(key).or_default().merge(demand);
        }
        changed
    }
}

fn conservative_type(graph: &FunctionGraph, ty: TyId, demand: Demand) -> Demand {
    if !demand.is_none()
        && (matches!(graph.oracle.shape(ty), TypeShape::Array { .. })
            || graph.oracle.semantic().maybe_uninit_inner(ty).is_some())
    {
        Demand::All
    } else {
        demand
    }
}

pub(super) fn analyze(graph: &FunctionGraph) -> HashMap<NodeId, Demand> {
    let mut demands: HashMap<_, _> = graph
        .ids()
        .filter(|&n| graph.rust_type(n).is_some())
        .map(|n| (n, Demand::None))
        .collect();
    let mut queue = Vec::new();
    let add = |node: NodeId,
               demand: Demand,
               demands: &mut HashMap<NodeId, Demand>,
               queue: &mut Vec<NodeId>| {
        let node = graph.resolve(node);
        let Some(ty) = graph.rust_type(node) else {
            return;
        };
        let demand = conservative_type(graph, ty, demand);
        if demands.entry(node).or_default().merge(&demand) {
            queue.push(node);
        }
    };
    // Unknown consumers request the full value. Pure structural operations
    // propagate only demands arriving from their own execution consumers.
    for node in graph.ids() {
        match graph.op(node) {
            Opcode::Aggregate
            | Opcode::WidePtr
            | Opcode::MakeVariant
            | Opcode::Repeat
            | Opcode::Field
            | Opcode::VariantField
            | Opcode::InsertField
            | Opcode::Phi
            | Opcode::VariantOf
            | Opcode::Discriminant
            | Opcode::VariantWitness
            | Opcode::Projection
            | Opcode::ConstRepr
            | Opcode::Undefined => {}
            Opcode::VariantSwitch => add(
                graph.input(node, 0),
                Demand::path(Key::Tag, Demand::All),
                &mut demands,
                &mut queue,
            ),
            _ => {
                for &input in graph.inputs(node) {
                    add(input, Demand::All, &mut demands, &mut queue);
                }
            }
        }
    }
    while let Some(node) = queue.pop() {
        let demand = demands[&node].clone();
        let inputs = graph.inputs(node);
        let mut propagate = |input, demand| add(input, demand, &mut demands, &mut queue);
        match graph.op(node) {
            Opcode::Field => propagate(
                inputs[0],
                Demand::path(Key::Field(graph.payload(node)), demand),
            ),
            Opcode::VariantField => {
                let (variant, field) = graph.variant_fields[graph.payload(node) as usize];
                let mut source = Demand::path(Key::VariantField(variant, field), demand);
                source.merge(&Demand::path(Key::Tag, Demand::All));
                propagate(inputs[0], source);
            }
            Opcode::VariantOf | Opcode::Discriminant => {
                propagate(inputs[0], Demand::path(Key::Tag, Demand::All))
            }
            Opcode::Aggregate | Opcode::WidePtr => {
                for (i, &input) in inputs.iter().enumerate() {
                    propagate(input, demand.child(Key::Field(i as u32)));
                }
            }
            Opcode::MakeVariant => {
                for (i, &input) in inputs.iter().enumerate() {
                    propagate(
                        input,
                        demand.child(Key::VariantField(graph.payload(node), i as u32)),
                    );
                }
            }
            Opcode::InsertField => {
                let field = graph.payload(node);
                propagate(inputs[1], demand.child(Key::Field(field)));
                // All minus one field requires a finite field shape. Arrays
                // retain their compact full representation in this first pass.
                let ty = graph.rust_type(node).unwrap();
                let base = if matches!(graph.oracle.shape(ty), TypeShape::Array { .. }) {
                    Demand::All
                } else if matches!(demand, Demand::All) {
                    let fields = graph
                        .oracle
                        .semantic()
                        .field_types(ty, None)
                        .unwrap_or_default();
                    Demand::Fields(
                        (0..fields.len() as u32)
                            .filter(|&i| i != field)
                            .map(|i| (Key::Field(i), Demand::All))
                            .collect(),
                    )
                } else if let Demand::Fields(mut fields) = demand {
                    fields.remove(&Key::Field(field));
                    if fields.is_empty() {
                        Demand::None
                    } else {
                        Demand::Fields(fields)
                    }
                } else {
                    Demand::None
                };
                propagate(inputs[0], base);
            }
            Opcode::Phi => {
                for &input in &inputs[1..] {
                    propagate(input, demand.clone());
                }
            }
            Opcode::Repeat => propagate(inputs[0], Demand::All),
            Opcode::Projection | Opcode::ConstRepr | Opcode::Undefined => {}
            _ => {
                for &input in inputs {
                    propagate(input, Demand::All);
                }
            }
        }
    }
    demands
}

impl LayoutLowering<'_> {
    pub(super) fn demand(&self, node: NodeId) -> Demand {
        self.demands.get(&node).cloned().unwrap_or(Demand::All)
    }
    pub(super) fn demand_for_type(&self, ty: TyId, demand: Demand) -> Demand {
        conservative_type(self.graph, ty, demand)
    }
    pub(super) fn omitted(&self, ty: TyId) -> Value {
        Value {
            ty,
            parts: Parts::Omitted,
        }
    }
}
