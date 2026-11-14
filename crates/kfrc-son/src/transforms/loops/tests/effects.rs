use super::*;
use kfrc_body::{Mutability, TypeOracle};

#[test]
fn fixed_observations_follow_the_original_control_and_effect_order() {
    let mut f = Fixture::new(IntegerWidth::W32, false, 0, 4, 1);
    f.fixed(Opcode::RequireDefined, f.a, false);
    let result = f.fixed(Opcode::Opaque, f.next_a, true).unwrap();
    f.graph.set_input(f.a, 2, result);
    f.verify();
    let before = f.graph.clone();
    let stats = run(&mut f.graph).unwrap();
    assert_eq!(stats.loops_unrolled, 1);
    for op in [Opcode::RequireDefined, Opcode::Opaque] {
        assert_eq!(f.graph.ids().filter(|&n| f.graph.op(n) == op).count(), 4);
    }
    f.verify();
    equivalent(&before, &f.graph, u32::MAX.into());
}

#[test]
fn zero_iterations_do_not_execute_a_body_poison_observation() {
    for count in [0, 1] {
        let mut f = Fixture::new(IntegerWidth::W32, false, 0, count, 1);
        let poison = f.graph.add_node(
            Opcode::Undefined,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(f.word),
        );
        f.fixed(Opcode::RequireDefined, poison, false);
        f.verify();
        let before = f.graph.clone();
        assert_eq!(run(&mut f.graph).unwrap().iterations_cloned, count as usize);
        f.verify();
        equivalent(&before, &f.graph, u32::MAX.into());
        assert_eq!(
            execute(&f.graph, &[Value::Scalar(1), Value::Scalar(2)]).is_ok(),
            count == 0
        );
    }
}

#[test]
fn non_induction_poison_flags_are_copied_without_strengthening_or_erasure() {
    let mut f = Fixture::new(IntegerWidth::W8, false, 0, 4, 1);
    f.graph.set_payload(f.next_a, ArithmeticFlags::NUW);
    f.verify();
    let before = f.graph.clone();
    assert_eq!(run(&mut f.graph).unwrap().loops_unrolled, 1);
    assert_eq!(
        f.graph
            .ids()
            .filter(|&node| {
                f.graph.op(node) == Opcode::Add && f.graph.payload(node) == ArithmeticFlags::NUW
            })
            .count(),
        4
    );
    f.verify();
    equivalent(&before, &f.graph, 255);
}

#[test]
fn single_predecessor_regions_and_phis_are_transparent_within_the_body() {
    let mut f = Fixture::new(IntegerWidth::W32, false, 0, 4, 1);
    let jump = f
        .graph
        .add_node(Opcode::Jump, f.body, &[], 0, ValueType::Control);
    let region = f
        .graph
        .add_node(Opcode::Region, NodeId::NONE, &[jump], 0, ValueType::Control);
    let local = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, f.b],
        0,
        ValueType::Rust(f.word),
    );
    f.graph.set_input(f.next_a, 0, local);
    f.graph.set_control(f.latch, region);
    f.verify();
    let before = f.graph.clone();
    assert_eq!(run(&mut f.graph).unwrap().loops_unrolled, 1);
    f.verify();
    equivalent(&before, &f.graph, u32::MAX.into());
}

fn indexed_memory(f: &mut Fixture) -> NodeId {
    let size = f.graph.oracle.size_align(f.word).unwrap();
    let array = f.graph.oracle.intern(
        TypeShape::Array {
            element: f.word,
            length: 4,
        },
        Some(SizeAlign {
            size: size.size * 4,
            align: size.align,
        }),
    );
    let array_ptr = f.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: array,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let word_ptr = f.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: f.word,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let slot = f.graph.add_node(
        Opcode::LocalSlot,
        NodeId::NONE,
        &[],
        91,
        ValueType::Rust(array_ptr),
    );
    let address = f.graph.add_node(
        Opcode::IndexAddr,
        NodeId::NONE,
        &[slot, f.iv],
        0,
        ValueType::Rust(word_ptr),
    );
    let store = f.graph.add_node(
        Opcode::StoreValue,
        f.body,
        &[f.effect, address, f.next_a],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    let control = projection(&mut f.graph, store, 0, ValueType::Control);
    let effect = projection(&mut f.graph, store, 1, ValueType::Effect);
    let load = f.graph.add_node(
        Opcode::LoadValue,
        control,
        &[effect, address],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(f.word),
        ]),
    );
    let control = projection(&mut f.graph, load, 0, ValueType::Control);
    let effect = projection(&mut f.graph, load, 1, ValueType::Effect);
    let value = projection(&mut f.graph, load, 2, ValueType::Rust(f.word));
    f.graph.set_control(f.latch, control);
    f.graph.set_input(f.effect, 2, effect);
    f.graph.set_input(f.a, 2, value);
    slot
}

#[test]
fn fixed_index_memory_reuses_one_allocation_and_reports_promotion_seeds() {
    let mut f = Fixture::new(IntegerWidth::W64, false, 0, 4, 1);
    let slot = indexed_memory(&mut f);
    f.verify();
    let before = f.graph.clone();
    let old_len = f.graph.len();
    let stats = run(&mut f.graph).unwrap();
    assert_eq!(stats.loops_unrolled, 1);
    assert_eq!(stats.nodes_cloned, f.graph.len() - old_len);
    assert_eq!(stats.exposed_slots, [slot]);
    assert_eq!(
        f.graph
            .ids()
            .filter(|&n| f.graph.op(n) == Opcode::LocalSlot)
            .count(),
        1
    );
    let addresses: Vec<_> = f
        .graph
        .ids()
        .filter(|&n| f.graph.op(n) == Opcode::IndexAddr)
        .collect();
    assert_eq!(addresses.len(), 4);
    for address in addresses {
        assert_eq!(f.graph.op(f.graph.input(address, 1)), Opcode::Const);
        assert!(stats.dirty_nodes.contains(&address));
    }
    f.verify();
    equivalent(&before, &f.graph, u64::MAX);
}

#[test]
fn narrow_signed_indices_do_not_cross_the_existing_s_to_l_semantic_seam() {
    let mut f = Fixture::new(IntegerWidth::W32, true, 0, 4, 1);
    indexed_memory(&mut f);
    unchanged(f, Limits::default());
}

#[test]
fn full_and_seeded_pipelines_both_expose_and_clean_up_complete_unrolling() {
    for seeded in [false, true] {
        let mut f = Fixture::new(IntegerWidth::W64, false, 0, 4, 1);
        f.verify();
        let before = f.graph.clone();
        let stats = if seeded {
            crate::optimize_specialized_seams(&mut f.graph, &[f.next_a], &[]).unwrap()
        } else {
            crate::optimize_specialized_graph(&mut f.graph).unwrap()
        };
        assert_eq!(stats.loops_unrolled, 1);
        assert_eq!(stats.loop_iterations_cloned, 4);
        assert!(stats.loop_nodes_cloned >= 4);
        assert!(
            !f.graph
                .ids()
                .any(|n| matches!(f.graph.op(n), Opcode::If | Opcode::Phi))
        );
        f.verify();
        equivalent(&before, &f.graph, u64::MAX);
    }
}
