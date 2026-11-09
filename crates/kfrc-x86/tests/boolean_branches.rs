//! A defined `bool` is 0 or 1: a branch on its negation tests the value
//! against zero with the inverse condition, without materializing `xor 1`.
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TypeShape};
use kfrc_son::{FunctionGraph, GraphLevel, NodeId, Opcode, ValueType, lower_layout, verify_graph};
use kfrc_x86::{MachineFunction, MachineInstr, lower_function};

fn projection(g: &mut FunctionGraph, source: NodeId, slot: u32, ty: ValueType) -> NodeId {
    g.add_node(Opcode::Projection, NodeId::NONE, &[source], slot, ty)
}

/// `fn(flag: bool) -> u64 { if !...!flag { 7 } else { 9 } }` with `nots`
/// negations; `shared` also returns the first negation's integer value.
fn fixture(nots: usize, shared: bool) -> MachineFunction {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let mut g = FunctionGraph::new(
        "boolean_branch",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    g.level = GraphLevel::Specialized;
    let start = g.add_node(
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
    g.start = start;
    let control = projection(&mut g, start, 0, ValueType::Control);
    let effect = projection(&mut g, start, 1, ValueType::Effect);
    let flag = projection(&mut g, start, 2, ValueType::Rust(boolean));
    let mut condition = flag;
    let mut first_negation = None;
    for _ in 0..nots {
        condition = g.add_node(
            Opcode::BitNot,
            NodeId::NONE,
            &[condition],
            0,
            ValueType::Rust(boolean),
        );
        first_negation.get_or_insert(condition);
    }
    let branch = g.add_node(
        Opcode::If,
        control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
    );
    let yes = projection(&mut g, branch, 0, ValueType::Control);
    let no = projection(&mut g, branch, 1, ValueType::Control);
    for (control, value) in [(yes, 7u128), (no, 9)] {
        let mut result = g.add_constant(value, word);
        if shared && let Some(negation) = first_negation {
            let widened = g.add_node(
                Opcode::IntCast,
                NodeId::NONE,
                &[negation],
                0,
                ValueType::Rust(word),
            );
            result = g.add_node(
                Opcode::Add,
                NodeId::NONE,
                &[result, widened],
                0,
                ValueType::Rust(word),
            );
        }
        g.add_node(
            Opcode::Return,
            control,
            &[effect, result],
            0,
            ValueType::Control,
        );
    }
    verify_graph(&mut g).unwrap_or_else(|error| panic!("{error}\n{}", g.dump()));
    lower_layout(&mut g).unwrap();
    verify_graph(&mut g).unwrap();
    lower_function(&g).unwrap_or_else(|error| panic!("{error}\n{}", g.dump()))
}

fn xors(machine: &MachineFunction) -> usize {
    machine
        .instructions
        .iter()
        .filter(|instruction| matches!(instruction, MachineInstr::Xor { .. }))
        .count()
}

#[test]
fn negated_boolean_branches_invert_the_condition_without_xor() {
    for nots in 0..4 {
        let machine = fixture(nots, false);
        assert_eq!(xors(&machine), 0, "{nots} negations: {machine:?}");
        for flag in [0, 1] {
            let taken = (flag == 1) ^ (nots % 2 == 1);
            assert_eq!(
                kfrc_x86::machine::eval::evaluate(&machine, &[flag]).unwrap(),
                if taken { 7 } else { 9 },
                "{nots} negations, flag {flag}"
            );
        }
    }
}

#[test]
fn shared_negation_stays_a_materialized_value() {
    let machine = fixture(1, true);
    assert!(xors(&machine) > 0);
    for flag in [0, 1] {
        let negation = 1 - flag;
        let expected = if negation == 1 { 7 } else { 9 } + negation;
        assert_eq!(
            kfrc_x86::machine::eval::evaluate(&machine, &[flag]).unwrap(),
            expected
        );
    }
}
