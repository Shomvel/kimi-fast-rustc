//! Graph-selected address operands stay live through allocation and native calls.
use kfrc_body::*;
use kfrc_son::*;
use kfrc_x86::machine::x86::Scale;
use kfrc_x86::{MachineFunction, MachineInstr, lower_function};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    narrow: TyId,
    pointer: TyId,
    control: NodeId,
    effect: NodeId,
    args: Vec<NodeId>,
}

fn projection(graph: &mut FunctionGraph, source: NodeId, index: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[source], index, ty)
}

impl Fixture {
    fn new(name: &str, arguments: usize) -> Self {
        let mut types = RecordedTypes::new();
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let narrow = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W32,
                signed: false,
            },
            Some(SizeAlign { size: 4, align: 4 }),
        );
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Mut,
                pointee: word,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut graph = FunctionGraph::new(
            name,
            FunctionAbi {
                args: vec![PassMode::Direct; arguments],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Layout;
        let mut outputs = vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(pointer),
        ];
        outputs.extend((1..arguments).map(|_| ValueType::Rust(word)));
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(outputs),
        );
        graph.start = start;
        let control = projection(&mut graph, start, 0, ValueType::Control);
        let effect = projection(&mut graph, start, 1, ValueType::Effect);
        let args = (0..arguments)
            .map(|index| {
                projection(
                    &mut graph,
                    start,
                    index as u32 + 2,
                    ValueType::Rust(if index == 0 { pointer } else { word }),
                )
            })
            .collect();
        Self {
            graph,
            word,
            narrow,
            pointer,
            control,
            effect,
            args,
        }
    }

    fn constant(&mut self, value: i64) -> NodeId {
        self.graph.add_constant(value as u64 as u128, self.word)
    }

    fn binary(&mut self, op: Opcode, lhs: NodeId, rhs: NodeId) -> NodeId {
        self.graph
            .add_node(op, NodeId::NONE, &[lhs, rhs], 0, ValueType::Rust(self.word))
    }

    fn scaled(&mut self, factor: i64, shift: bool) -> NodeId {
        let constant = self.constant(if shift {
            factor.trailing_zeros() as i64
        } else {
            factor
        });
        self.binary(
            if shift {
                Opcode::ShiftLeft
            } else {
                Opcode::Mul
            },
            self.args[1],
            constant,
        )
    }

    fn address(&mut self, index: NodeId, displacement: i64) -> NodeId {
        let index = if displacement == 0 {
            index
        } else {
            let offset = self.constant(displacement);
            self.binary(Opcode::Add, index, offset)
        };
        self.graph.add_node(
            Opcode::PtrOffset,
            NodeId::NONE,
            &[self.args[0], index],
            0,
            ValueType::Rust(self.pointer),
        )
    }

    fn load(&mut self, address: NodeId, ty: TyId) -> NodeId {
        let load = self.graph.add_node(
            Opcode::Load,
            self.control,
            &[self.effect, address],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(ty),
            ]),
        );
        self.control = projection(&mut self.graph, load, 0, ValueType::Control);
        self.effect = projection(&mut self.graph, load, 1, ValueType::Effect);
        projection(&mut self.graph, load, 2, ValueType::Rust(ty))
    }

    fn store(&mut self, address: NodeId, value: NodeId) {
        let store = self.graph.add_node(
            Opcode::Store,
            self.control,
            &[self.effect, address, value],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        self.control = projection(&mut self.graph, store, 0, ValueType::Control);
        self.effect = projection(&mut self.graph, store, 1, ValueType::Effect);
    }

    fn cast(&mut self, value: NodeId, ty: TyId) -> NodeId {
        self.graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[value],
            0,
            ValueType::Rust(ty),
        )
    }

    fn finish(mut self, result: NodeId) -> MachineFunction {
        self.graph.add_node(
            Opcode::Return,
            self.control,
            &[self.effect, result],
            0,
            ValueType::Control,
        );
        verify_graph(&mut self.graph)
            .unwrap_or_else(|error| panic!("{error}\n{}", self.graph.dump()));
        lower_function(&self.graph).unwrap_or_else(|error| panic!("{error}\n{}", self.graph.dump()))
    }
}

fn indexed_loads(function: &MachineFunction) -> Vec<(Scale, i32)> {
    function
        .instructions
        .iter()
        .filter_map(|instruction| match instruction {
            MachineInstr::LoadAddress { address, .. } => {
                Some((address.index.unwrap().1, address.offset))
            }
            _ => None,
        })
        .collect()
}

fn address_arithmetic(function: &MachineFunction) -> usize {
    function
        .instructions
        .iter()
        .filter(|instruction| {
            matches!(
                instruction,
                MachineInstr::Add { .. }
                    | MachineInstr::OffsetAddress { .. }
                    | MachineInstr::IMul { .. }
                    | MachineInstr::Shl { .. }
            )
        })
        .count()
}

#[test]
fn scales_and_displacements_fold_without_materializing_consumed_arithmetic() {
    for (factor, scale) in [
        (1, Scale::One),
        (2, Scale::Two),
        (4, Scale::Four),
        (8, Scale::Eight),
    ] {
        for shift in [false, true] {
            for displacement in [i32::MIN, -129, -128, 0, 127, 128, i32::MAX] {
                let mut f = Fixture::new("scaled", 2);
                let index = f.scaled(factor, shift);
                let address = f.address(index, i64::from(displacement));
                let value = f.load(address, f.word);
                let machine = f.finish(value);
                assert_eq!(indexed_loads(&machine), [(scale, displacement)]);
                assert_eq!(
                    address_arithmetic(&machine),
                    0,
                    "{:?}",
                    machine.instructions
                );
                machine.emit_machine_code().unwrap();
            }
        }
    }
}

#[test]
fn arithmetic_needed_as_data_is_materialized() {
    for escape_pointer in [false, true] {
        let mut f = Fixture::new("multiuse", 2);
        let index = f.scaled(4, false);
        let address = f.address(index, 16);
        f.store(f.args[0], if escape_pointer { address } else { index });
        let value = f.load(address, f.word);
        let machine = f.finish(value);
        assert_eq!(indexed_loads(&machine), [(Scale::Four, 16)]);
        assert!(
            address_arithmetic(&machine) > 0,
            "{:?}",
            machine.instructions
        );
    }
}

fn narrow_index() -> MachineFunction {
    let mut f = Fixture::new("sib_narrow", 2);
    let input = f.cast(f.args[1], f.narrow);
    let four = f.graph.add_constant(4, f.narrow);
    let product = f.graph.add_node(
        Opcode::Mul,
        NodeId::NONE,
        &[input, four],
        0,
        ValueType::Rust(f.narrow),
    );
    let wide = f.cast(product, f.word);
    let address = f.address(wide, 0);
    let value = f.load(address, f.word);
    f.finish(value)
}

#[test]
fn narrow_wrapping_and_anchored_arithmetic_keep_their_computation() {
    let narrow = narrow_index();
    assert_eq!(indexed_loads(&narrow), [(Scale::One, 0)]);
    assert!(address_arithmetic(&narrow) > 0);
    let mut f = Fixture::new("anchored", 2);
    let four = f.constant(4);
    let index = f.graph.add_node(
        Opcode::Mul,
        f.control,
        &[f.args[1], four],
        0,
        ValueType::Rust(f.word),
    );
    let address = f.address(index, 0);
    let value = f.load(address, f.word);
    let machine = f.finish(value);
    assert_eq!(indexed_loads(&machine), [(Scale::One, 0)]);
    assert!(address_arithmetic(&machine) > 0);
}

#[test]
fn a_second_dynamic_index_and_large_displacement_keep_the_unfolded_part() {
    let mut f = Fixture::new("two_indices", 3);
    let first = f.scaled(4, false);
    let address = f.address(first, 0);
    let address = f.graph.add_node(
        Opcode::PtrOffset,
        NodeId::NONE,
        &[address, f.args[2]],
        0,
        ValueType::Rust(f.pointer),
    );
    let value = f.load(address, f.word);
    let machine = f.finish(value);
    assert_eq!(indexed_loads(&machine), [(Scale::One, 0)]);
    assert!(address_arithmetic(&machine) >= 2);

    let mut f = Fixture::new("large_displacement", 2);
    let index = f.scaled(8, false);
    let address = f.address(index, i64::from(i32::MAX) + 1);
    let value = f.load(address, f.word);
    let machine = f.finish(value);
    assert_eq!(indexed_loads(&machine), [(Scale::One, 0)]);
    assert!(address_arithmetic(&machine) >= 2);
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "indexed_address_modes/native.rs"]
mod native;
