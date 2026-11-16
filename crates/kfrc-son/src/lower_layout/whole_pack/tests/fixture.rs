use super::*;
use kfrc_body::{FunctionAbi, Mutability, PassMode, RecordedTypes};

pub(super) const SENTINEL: u64 = 0xbad0_dead_face_f00d;
const BYTES: [u8; 8] = [0x98, 0xba, 0xdc, 0xfe, 0x01, 0x7e, 0xff, 0x80];

pub(super) struct Fixture {
    pub graph: FunctionGraph,
    pub byte: TyId,
    pub word: TyId,
    pub pointer: TyId,
    pub base: NodeId,
    pub input_effect: NodeId,
    pub region: NodeId,
    pub effect_phi: NodeId,
    pub loads: Vec<NodeId>,
    pub addresses: Vec<NodeId>,
    pub phis: Vec<NodeId>,
    pub casts: Vec<NodeId>,
    pub terms: Vec<NodeId>,
    pub pack: NodeId,
    pub ret: NodeId,
    pub active: NodeId,
    pub bytes: usize,
    pub slot: usize,
}

impl Fixture {
    pub fn new(
        bytes: usize,
        slot: usize,
        balanced: bool,
        missing: Option<usize>,
        misalign: u64,
    ) -> Self {
        let mut types = RecordedTypes::new();
        let byte = integer(&mut types, 1);
        let word = integer(&mut types, bytes as u64);
        let address_word = integer(&mut types, 8);
        let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Mut,
                pointee: byte,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut graph = FunctionGraph::new(
            "guarded_whole_pack",
            FunctionAbi {
                args: vec![PassMode::Direct],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Layout;
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(boolean),
            ]),
        );
        graph.start = start;
        let mut control = project(&mut graph, start, 0, ValueType::Control);
        let mut effect = project(&mut graph, start, 1, ValueType::Effect);
        let active = project(&mut graph, start, 2, ValueType::Rust(boolean));
        graph.stack_slots.push(SizeAlign {
            size: bytes as u64 + misalign,
            align: 8,
        });
        let allocation = graph.add_node(
            Opcode::StackSlot,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(pointer),
        );
        let offset = graph.add_constant(u128::from(misalign), address_word);
        let base = graph.add_node(
            Opcode::PtrOffset,
            NodeId::NONE,
            &[allocation, offset],
            0,
            ValueType::Rust(pointer),
        );
        for (offset, &bits) in BYTES[..bytes].iter().enumerate() {
            if missing == Some(offset) {
                continue;
            }
            let offset = graph.add_constant(offset as u128, address_word);
            let address = graph.add_node(
                Opcode::PtrOffset,
                NodeId::NONE,
                &[base, offset],
                0,
                ValueType::Rust(pointer),
            );
            let value = graph.add_constant(u128::from(bits), byte);
            let store = graph.add_node(
                Opcode::Store,
                control,
                &[effect, address, value],
                0,
                ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
            );
            control = project(&mut graph, store, 0, ValueType::Control);
            effect = project(&mut graph, store, 1, ValueType::Effect);
        }
        let input_effect = effect;
        let branch = graph.add_node(
            Opcode::If,
            control,
            &[active],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        let yes = project(&mut graph, branch, 0, ValueType::Control);
        let no = project(&mut graph, branch, 1, ValueType::Control);
        control = yes;
        let mut loads = Vec::new();
        let mut values = Vec::new();
        let mut addresses = Vec::new();
        for offset in 0..bytes {
            let offset = graph.add_constant(offset as u128, address_word);
            let address = graph.add_node(
                Opcode::PtrOffset,
                NodeId::NONE,
                &[base, offset],
                0,
                ValueType::Rust(pointer),
            );
            let load = graph.add_node(
                Opcode::Load,
                control,
                &[effect, address],
                0,
                ValueType::Tuple(vec![
                    ValueType::Control,
                    ValueType::Effect,
                    ValueType::Rust(byte),
                ]),
            );
            control = project(&mut graph, load, 0, ValueType::Control);
            effect = project(&mut graph, load, 1, ValueType::Effect);
            values.push(project(&mut graph, load, 2, ValueType::Rust(byte)));
            loads.push(load);
            addresses.push(address);
        }
        let yes_jump = graph.add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
        let no_jump = graph.add_node(Opcode::Jump, no, &[], 0, ValueType::Control);
        let edges = if slot == 1 {
            [yes_jump, no_jump]
        } else {
            [no_jump, yes_jump]
        };
        let region = graph.add_node(Opcode::Region, NodeId::NONE, &edges, 0, ValueType::Control);
        let mut effects = [region, input_effect, input_effect];
        effects[slot] = effect;
        let effect_phi = graph.add_node(Opcode::Phi, NodeId::NONE, &effects, 0, ValueType::Effect);
        let mut phis = Vec::new();
        let mut casts = Vec::new();
        let mut terms = Vec::new();
        for (index, value) in values.into_iter().enumerate() {
            let undefined = graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(byte),
            );
            let mut inputs = [region, undefined, undefined];
            inputs[slot] = value;
            let phi = graph.add_node(Opcode::Phi, NodeId::NONE, &inputs, 0, ValueType::Rust(byte));
            let cast = graph.add_node(
                Opcode::IntCast,
                NodeId::NONE,
                &[phi],
                0,
                ValueType::Rust(word),
            );
            let term = if index == 0 {
                cast
            } else {
                let shift = graph.add_constant((index * 8) as u128, word);
                graph.add_node(
                    Opcode::ShiftLeft,
                    NodeId::NONE,
                    &[cast, shift],
                    0,
                    ValueType::Rust(word),
                )
            };
            phis.push(phi);
            casts.push(cast);
            terms.push(term);
        }
        let pack = pack_tree(&mut graph, &terms, word, balanced);
        let branch = graph.add_node(
            Opcode::If,
            region,
            &[active],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        let yes = project(&mut graph, branch, 0, ValueType::Control);
        let no = project(&mut graph, branch, 1, ValueType::Control);
        let returned = graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[pack],
            0,
            ValueType::Rust(address_word),
        );
        let ret = graph.add_node(
            Opcode::Return,
            yes,
            &[effect_phi, returned],
            0,
            ValueType::Control,
        );
        let sentinel = graph.add_constant(u128::from(SENTINEL), address_word);
        graph.add_node(
            Opcode::Return,
            no,
            &[effect_phi, sentinel],
            0,
            ValueType::Control,
        );
        Self {
            graph,
            byte,
            word,
            pointer,
            base,
            input_effect,
            region,
            effect_phi,
            loads,
            addresses,
            phis,
            casts,
            terms,
            pack,
            ret,
            active,
            bytes,
            slot,
        }
    }

    pub fn expected(&self) -> u64 {
        let mut bytes = [0; 8];
        bytes[..self.bytes].copy_from_slice(&BYTES[..self.bytes]);
        u64::from_le_bytes(bytes)
    }

    pub fn append_second_group(&mut self) {
        let final_branch = self.graph.input(self.graph.control(self.ret), 0);
        let branch = self.graph.add_node(
            Opcode::If,
            self.region,
            &[self.active],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        let mut control = project(&mut self.graph, branch, 0, ValueType::Control);
        let no = project(&mut self.graph, branch, 1, ValueType::Control);
        let mut effect = self.effect_phi;
        let mut values = Vec::new();
        for &address in &self.addresses {
            let load = self.graph.add_node(
                Opcode::Load,
                control,
                &[effect, address],
                0,
                ValueType::Tuple(vec![
                    ValueType::Control,
                    ValueType::Effect,
                    ValueType::Rust(self.byte),
                ]),
            );
            control = project(&mut self.graph, load, 0, ValueType::Control);
            effect = project(&mut self.graph, load, 1, ValueType::Effect);
            values.push(project(
                &mut self.graph,
                load,
                2,
                ValueType::Rust(self.byte),
            ));
        }
        let yes = self
            .graph
            .add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
        let no = self
            .graph
            .add_node(Opcode::Jump, no, &[], 0, ValueType::Control);
        let region = self.graph.add_node(
            Opcode::Region,
            NodeId::NONE,
            &[yes, no],
            0,
            ValueType::Control,
        );
        let effect = self.graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[region, effect, self.effect_phi],
            0,
            ValueType::Effect,
        );
        let mut terms = Vec::new();
        for (index, value) in values.into_iter().enumerate() {
            let poison = self.graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(self.byte),
            );
            let phi = self.graph.add_node(
                Opcode::Phi,
                NodeId::NONE,
                &[region, value, poison],
                0,
                ValueType::Rust(self.byte),
            );
            let cast = self.graph.add_node(
                Opcode::IntCast,
                NodeId::NONE,
                &[phi],
                0,
                ValueType::Rust(self.word),
            );
            let term = if index == 0 {
                cast
            } else {
                let shift = self.graph.add_constant((index * 8) as u128, self.word);
                self.graph.add_node(
                    Opcode::ShiftLeft,
                    NodeId::NONE,
                    &[cast, shift],
                    0,
                    ValueType::Rust(self.word),
                )
            };
            terms.push(term);
        }
        let second = pack_tree(&mut self.graph, &terms, self.word, true);
        let sum = self.graph.add_node(
            Opcode::Add,
            NodeId::NONE,
            &[self.pack, second],
            0,
            ValueType::Rust(self.word),
        );
        let returned = self.graph.input(self.ret, 1);
        self.graph.set_input(returned, 0, sum);
        self.graph.set_control(final_branch, region);
        let returns: Vec<_> = self
            .graph
            .ids()
            .filter(|&node| self.graph.op(node) == Opcode::Return)
            .collect();
        for ret in returns {
            self.graph.set_input(ret, 0, effect);
        }
    }

    pub fn overwrite_after_join(&mut self) {
        let zero = self.graph.add_constant(0, self.byte);
        let control = self.graph.control(self.ret);
        let store = self.graph.add_node(
            Opcode::Store,
            control,
            &[self.effect_phi, self.base, zero],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        let control = project(&mut self.graph, store, 0, ValueType::Control);
        let effect = project(&mut self.graph, store, 1, ValueType::Effect);
        self.graph.set_control(self.ret, control);
        self.graph.set_input(self.ret, 0, effect);
    }
}

pub(super) fn project(
    graph: &mut FunctionGraph,
    source: NodeId,
    slot: u32,
    ty: ValueType,
) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[source], slot, ty)
}

fn integer(types: &mut RecordedTypes, bytes: u64) -> TyId {
    let width = match bytes {
        1 => IntegerWidth::W8,
        2 => IntegerWidth::W16,
        4 => IntegerWidth::W32,
        8 => IntegerWidth::W64,
        _ => unreachable!(),
    };
    types.intern(
        TypeShape::Int {
            width,
            signed: false,
        },
        Some(SizeAlign {
            size: bytes,
            align: bytes,
        }),
    )
}

fn pack_tree(graph: &mut FunctionGraph, terms: &[NodeId], ty: TyId, balanced: bool) -> NodeId {
    if terms.len() == 1 {
        return terms[0];
    }
    let split = if balanced {
        terms.len() / 2
    } else {
        terms.len() - 1
    };
    let left = pack_tree(graph, &terms[..split], ty, balanced);
    let right = pack_tree(graph, &terms[split..], ty, balanced);
    graph.add_node(
        Opcode::BitOr,
        NodeId::NONE,
        &[left, right],
        0,
        ValueType::Rust(ty),
    )
}
