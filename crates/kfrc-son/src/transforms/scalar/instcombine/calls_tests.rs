use super::*;
use crate::{CallTarget, GraphLevel, InterpretError, Interpreter, Outcome, Value, verify_graph};
use kfrc_body::{
    DataObject, FunctionAbi, Mutability, PassMode, RecordedTypes, ScalarValidRange, ScalarValidity,
    SizeAlign,
};

struct Fixture {
    graph: FunctionGraph,
    control: NodeId,
    effect: NodeId,
    flag: NodeId,
    pointers: [NodeId; 2],
    length: NodeId,
    integer: TyId,
    boolean: TyId,
}

impl Fixture {
    fn new(left: &[u8], right: &[u8], length: u64) -> Self {
        let mut types = RecordedTypes::new();
        let byte = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            Some(SizeAlign { size: 1, align: 1 }),
        );
        let integer = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W32,
                signed: true,
            },
            Some(SizeAlign { size: 4, align: 4 }),
        );
        let index = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Not,
                pointee: byte,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let mut graph = FunctionGraph::new(
            "memcmp_zero",
            FunctionAbi {
                args: vec![PassMode::Direct],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Layout;
        graph.start = graph.add_node(
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
        let mut projection =
            |index, ty| graph.add_node(Opcode::Projection, NodeId::NONE, &[graph.start], index, ty);
        let control = projection(0, ValueType::Control);
        let effect = projection(1, ValueType::Effect);
        let flag = projection(2, ValueType::Rust(boolean));
        let pointers = [left, right].map(|bytes| {
            let index = graph.symbols.len() as u32;
            let symbol = format!("bytes_{index}");
            graph.symbols.push((symbol.clone(), 0));
            graph.data_objects.push(DataObject {
                symbol,
                bytes: bytes.to_vec(),
                align: 1,
                ..Default::default()
            });
            graph.add_node(
                Opcode::SymbolAddress,
                NodeId::NONE,
                &[],
                index,
                ValueType::Rust(pointer),
            )
        });
        let length = graph.add_constant(u128::from(length), index);
        Self {
            graph,
            control,
            effect,
            flag,
            pointers,
            length,
            integer,
            boolean,
        }
    }

    fn call(&mut self, trusted: bool) -> [NodeId; 4] {
        let target = if trusted {
            self.graph.add_runtime_comparison(RuntimeComparison::Memcmp)
        } else {
            self.graph.add_callee(CallTarget {
                symbol: "memcmp".into(),
                abi: RuntimeComparison::Memcmp.abi(),
            })
        };
        let call = self.graph.add_node(
            Opcode::Call,
            self.control,
            &[self.effect, self.pointers[0], self.pointers[1], self.length],
            target,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(self.integer),
            ]),
        );
        let control = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[call],
            0,
            ValueType::Control,
        );
        let effect = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[call],
            1,
            ValueType::Effect,
        );
        let result = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[call],
            2,
            ValueType::Rust(self.integer),
        );
        [call, control, effect, result]
    }

    fn compare(
        &mut self,
        value: NodeId,
        predicate: ComparePredicate,
        constant: u64,
        swapped: bool,
    ) -> NodeId {
        let constant = self.graph.add_constant(u128::from(constant), self.integer);
        let inputs = if swapped {
            [constant, value]
        } else {
            [value, constant]
        };
        self.graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &inputs,
            predicate as u32,
            ValueType::Rust(self.boolean),
        )
    }

    fn finish(&mut self, control: NodeId, effect: NodeId, result: NodeId) {
        self.graph.add_node(
            Opcode::Return,
            control,
            &[effect, result],
            0,
            ValueType::Control,
        );
        verify_graph(&mut self.graph).unwrap();
    }

    fn reject(&mut self) {
        let before = self.graph.dump();
        let callees = self.graph.callees.clone();
        assert_eq!(run(&mut self.graph), 0);
        assert_eq!(self.graph.dump(), before);
        assert_eq!(self.graph.callees, callees);
    }

    fn rewrite(&mut self, call: NodeId) -> FunctionGraph {
        let before = self.graph.clone();
        assert_eq!(run(&mut self.graph), 1);
        assert_eq!(
            self.graph.runtime_comparison(self.graph.payload(call)),
            Some(RuntimeComparison::Bcmp)
        );
        // Authentication changes only the target. Exact IDs, fixed edges,
        // operands and projection uses retain argument/error observations.
        assert_eq!(self.graph.len(), before.len());
        for node in before.ids() {
            assert_eq!(self.graph.op(node), before.op(node));
            assert_eq!(self.graph.control(node), before.control(node));
            assert_eq!(self.graph.inputs(node), before.inputs(node));
            assert_eq!(self.graph.value_type(node), before.value_type(node));
            if node != call {
                assert_eq!(self.graph.payload(node), before.payload(node));
            }
        }
        verify_graph(&mut self.graph).unwrap();
        assert_eq!(run(&mut self.graph), 0, "already converted bcmp is stable");
        before
    }
}

fn execute(graph: &FunctionGraph, flag: u64) -> Result<Outcome, InterpretError> {
    Interpreter::new(graph).run(&[flag], &mut |_: &str, _: &[u64]| unreachable!())
}

fn execute_semantic(
    graph: &FunctionGraph,
    flag: u64,
    equal: bool,
) -> (Result<crate::SemanticOutcome, InterpretError>, usize) {
    let mut graph = graph.clone();
    graph.level = GraphLevel::Specialized;
    verify_graph(&mut graph).unwrap();
    let mut calls = 0;
    let outcome = Interpreter::new(&graph).run_values(
        &[Value::Scalar(flag)],
        &mut |symbol: &str, _: &[Value]| {
            calls += 1;
            // Valid runtime results can differ in every nonzero bit. This
            // independently exercises the zero-use proof; the L interpreter
            // returns the same first-byte ordering for both symbols.
            Ok(Value::Scalar(match (symbol, equal) {
                ("memcmp" | "bcmp", true) => 0,
                ("memcmp", false) => 0xffff_ffed,
                ("bcmp", false) => 1,
                _ => unreachable!(),
            }))
        },
    );
    (outcome, calls)
}

#[test]
fn authenticated_direct_zero_tests_preserve_equal_and_both_orderings() {
    for right in [&b"same"[..], &b"tame"[..], &b"rame"[..]] {
        for length in [0, 1, 4] {
            for predicate in [ComparePredicate::Equal, ComparePredicate::NotEqual] {
                for swapped in [false, true] {
                    let mut f = Fixture::new(b"same", right, length);
                    let [call, control, effect, result] = f.call(true);
                    let compared = f.compare(result, predicate, 0, swapped);
                    f.finish(control, effect, compared);
                    let before = f.rewrite(call);
                    let equal = b"same"[..length as usize] == right[..length as usize];
                    let expected = u64::from(equal == (predicate == ComparePredicate::Equal));
                    assert_eq!(execute(&before, 0), Ok(Outcome::Return(expected)));
                    assert_eq!(execute(&f.graph, 0), execute(&before, 0));
                    assert_eq!(
                        execute_semantic(&before, 0, equal),
                        execute_semantic(&f.graph, 0, equal)
                    );
                }
            }
        }
    }
}

#[test]
fn phi_zero_use_closure_preserves_both_paths_and_slot_order() {
    for right in [&b"same"[..], &b"tame"[..], &b"rame"[..]] {
        for other_bits in [0, 0xffff_fffd] {
            for reversed in [false, true] {
                let mut f = Fixture::new(b"same", right, 4);
                let branch = f.graph.add_node(
                    Opcode::If,
                    f.control,
                    &[f.flag],
                    0,
                    ValueType::Tuple(vec![ValueType::Control; 2]),
                );
                let yes = f.graph.add_node(
                    Opcode::Projection,
                    NodeId::NONE,
                    &[branch],
                    0,
                    ValueType::Control,
                );
                let no = f.graph.add_node(
                    Opcode::Projection,
                    NodeId::NONE,
                    &[branch],
                    1,
                    ValueType::Control,
                );
                f.control = yes;
                let [call, control, effect, result] = f.call(true);
                let call_jump = f
                    .graph
                    .add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
                let bypass_jump = f
                    .graph
                    .add_node(Opcode::Jump, no, &[], 0, ValueType::Control);
                let other = f.graph.add_constant(other_bits, f.integer);
                let (edges, values, effects) = if reversed {
                    (
                        [bypass_jump, call_jump],
                        [other, result],
                        [f.effect, effect],
                    )
                } else {
                    (
                        [call_jump, bypass_jump],
                        [result, other],
                        [effect, f.effect],
                    )
                };
                let region =
                    f.graph
                        .add_node(Opcode::Region, NodeId::NONE, &edges, 0, ValueType::Control);
                let phi = f.graph.add_node(
                    Opcode::Phi,
                    NodeId::NONE,
                    &[region, values[0], values[1]],
                    0,
                    ValueType::Rust(f.integer),
                );
                let effect = f.graph.add_node(
                    Opcode::Phi,
                    NodeId::NONE,
                    &[region, effects[0], effects[1]],
                    0,
                    ValueType::Effect,
                );
                let eq = f.compare(phi, ComparePredicate::Equal, 0, false);
                let ne = f.compare(phi, ComparePredicate::NotEqual, 0, true);
                let not_ne = f.graph.add_node(
                    Opcode::BitNot,
                    NodeId::NONE,
                    &[ne],
                    0,
                    ValueType::Rust(f.boolean),
                );
                let both = f.graph.add_node(
                    Opcode::BitAnd,
                    NodeId::NONE,
                    &[eq, not_ne],
                    0,
                    ValueType::Rust(f.boolean),
                );
                f.finish(region, effect, both);
                let before = f.rewrite(call);
                for flag in [0, 1] {
                    let equal = if flag == 1 {
                        b"same" == right
                    } else {
                        other_bits == 0
                    };
                    assert_eq!(
                        execute(&before, flag),
                        Ok(Outcome::Return(u64::from(equal)))
                    );
                    assert_eq!(execute(&f.graph, flag), execute(&before, flag));
                    let expected_calls = usize::from(flag == 1);
                    let expected = execute_semantic(&before, flag, b"same" == right);
                    assert_eq!(expected.1, expected_calls);
                    assert_eq!(expected, execute_semantic(&f.graph, flag, b"same" == right));
                }
            }
        }
    }
}

#[test]
fn zero_length_still_observes_each_poison_argument_at_the_original_call() {
    for poisoned in 1..=3 {
        let mut f = Fixture::new(b"", b"", 0);
        let [call, control, effect, result] = f.call(true);
        let input = f.graph.input(call, poisoned);
        let poison = f.graph.add_node(
            Opcode::Undefined,
            NodeId::NONE,
            &[],
            0,
            f.graph.value_type(input).clone(),
        );
        f.graph.set_input(call, poisoned, poison);
        let compared = f.compare(result, ComparePredicate::Equal, 0, false);
        f.finish(control, effect, compared);
        let before = f.graph.clone();
        if poisoned == 3 {
            // Unknown lengths retain memcmp; poison is still observed at
            // that same call. Constant zero retains both pointer arguments.
            f.reject();
        } else {
            f.rewrite(call);
        }
        assert_eq!(
            execute(&before, 0),
            Err(InterpretError::PoisonObserved(call))
        );
        assert_eq!(execute(&f.graph, 0), execute(&before, 0));
        // S's call boundary also checks every argument before invoking its
        // handler; the handler must not run on either authenticated target.
        for mut graph in [before, f.graph.clone()] {
            graph.level = GraphLevel::Specialized;
            verify_graph(&mut graph).unwrap();
            let observed = Interpreter::new(&graph).run_values(
                &[Value::Scalar(0)],
                &mut |_: &str, _: &[Value]| unreachable!(),
            );
            assert_eq!(observed, Err(InterpretError::PoisonObserved(call)));
        }
    }
}

#[test]
fn mixed_numeric_ordering_nonzero_and_raw_return_uses_are_rejected() {
    for use_kind in 0..5 {
        let mut f = Fixture::new(b"same", b"rame", 4);
        let [_, control, effect, result] = f.call(true);
        let returned = match use_kind {
            0 => result,
            1 => f.compare(result, ComparePredicate::SignedLess, 0, false),
            2 => f.compare(result, ComparePredicate::Equal, 1, false),
            3 => {
                let eq = f.compare(result, ComparePredicate::Equal, 0, false);
                let one = f.graph.add_constant(1, f.integer);
                let increment = f.graph.add_node(
                    Opcode::Add,
                    NodeId::NONE,
                    &[result, one],
                    0,
                    ValueType::Rust(f.integer),
                );
                let other = f.compare(increment, ComparePredicate::Equal, 0, false);
                f.graph.add_node(
                    Opcode::BitAnd,
                    NodeId::NONE,
                    &[eq, other],
                    0,
                    ValueType::Rust(f.boolean),
                )
            }
            _ => {
                let jump = f
                    .graph
                    .add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
                let region =
                    f.graph
                        .add_node(Opcode::Region, NodeId::NONE, &[jump], 0, ValueType::Control);
                let phi = f.graph.add_node(
                    Opcode::Phi,
                    NodeId::NONE,
                    &[region, result],
                    0,
                    ValueType::Rust(f.integer),
                );
                let eq = f.compare(phi, ComparePredicate::Equal, 0, false);
                let one = f.graph.add_constant(1, f.integer);
                let increment = f.graph.add_node(
                    Opcode::Add,
                    NodeId::NONE,
                    &[phi, one],
                    0,
                    ValueType::Rust(f.integer),
                );
                let other = f.compare(increment, ComparePredicate::Equal, 0, false);
                let returned = f.graph.add_node(
                    Opcode::BitAnd,
                    NodeId::NONE,
                    &[eq, other],
                    0,
                    ValueType::Rust(f.boolean),
                );
                f.finish(region, effect, returned);
                f.reject();
                continue;
            }
        };
        f.finish(control, effect, returned);
        f.reject();
    }
}

#[test]
fn dynamic_and_large_lengths_keep_the_original_runtime_target() {
    for length in [33, 64, u64::MAX] {
        let mut f = Fixture::new(b"same", b"same", length);
        let [_, control, effect, result] = f.call(true);
        let compared = f.compare(result, ComparePredicate::Equal, 0, false);
        f.finish(control, effect, compared);
        f.reject();
    }
    let mut f = Fixture::new(b"same", b"same", 4);
    let [call, control, effect, result] = f.call(true);
    let length = f.graph.add_node(
        Opcode::IntCast,
        NodeId::NONE,
        &[f.flag],
        0,
        f.graph.value_type(f.length).clone(),
    );
    f.graph.set_input(call, 3, length);
    let compared = f.compare(result, ComparePredicate::Equal, 0, false);
    f.finish(control, effect, compared);
    f.reject();
}

#[test]
fn same_name_calls_and_changed_runtime_abi_never_gain_authentication() {
    for ordinary_first in [false, true] {
        let mut f = Fixture::new(b"same", b"same", 4);
        let ordinary = CallTarget {
            symbol: "memcmp".into(),
            abi: RuntimeComparison::Memcmp.abi(),
        };
        let (plain, trusted) = if ordinary_first {
            let plain = f.graph.add_callee(ordinary.clone());
            (
                plain,
                f.graph.add_runtime_comparison(RuntimeComparison::Memcmp),
            )
        } else {
            let trusted = f.graph.add_runtime_comparison(RuntimeComparison::Memcmp);
            (f.graph.add_callee(ordinary.clone()), trusted)
        };
        assert_ne!(plain, trusted);
        assert_eq!(f.graph.add_callee(ordinary), plain);
        assert_eq!(
            f.graph.add_runtime_comparison(RuntimeComparison::Memcmp),
            trusted
        );
        assert_eq!(f.graph.runtime_comparison(plain), None);
        let [_, control, effect, result] = f.call(false);
        let compared = f.compare(result, ComparePredicate::Equal, 0, false);
        f.finish(control, effect, compared);
        f.reject();
    }
    for change in 0..4 {
        let mut f = Fixture::new(b"same", b"same", 4);
        let [call, control, effect, result] = f.call(true);
        let target = f.graph.payload(call) as usize;
        match change {
            0 => f.graph.callees[target].abi.args[0] = PassMode::Ignore,
            1 => f.graph.callees[target].abi.ret = PassMode::Ignore,
            2 => f.graph.callees[target].abi.c_variadic = true,
            _ => f.graph.callees[target].symbol = "user_memcmp".into(),
        }
        assert_eq!(f.graph.runtime_comparison(target as u32), None);
        let compared = f.compare(result, ComparePredicate::Equal, 0, false);
        // Ignore-return disagrees with this result tuple. Exercise that
        // malformed ABI only at the defensive matcher boundary.
        if change != 1 {
            f.finish(control, effect, compared);
        }
        f.reject();
    }
}

#[test]
fn a_verified_loop_phi_cycle_is_not_an_all_use_proof() {
    let mut f = Fixture::new(b"same", b"same", 4);
    let [_, control, effect, result] = f.call(true);
    let entry = f
        .graph
        .add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
    let region = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry, entry],
        0,
        ValueType::Control,
    );
    let phi = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, result, result],
        0,
        ValueType::Rust(f.integer),
    );
    let branch = f.graph.add_node(
        Opcode::If,
        region,
        &[f.flag],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let back = f.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[branch],
        0,
        ValueType::Control,
    );
    let exit = f.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[branch],
        1,
        ValueType::Control,
    );
    let jump = f
        .graph
        .add_node(Opcode::Jump, back, &[], 0, ValueType::Control);
    f.graph.set_input(region, 1, jump);
    f.graph.set_input(phi, 2, phi);
    let compared = f.compare(phi, ComparePredicate::Equal, 0, false);
    f.finish(exit, effect, compared);
    assert_eq!(execute(&f.graph, 0), Ok(Outcome::Return(1)));
    f.reject();
}

#[test]
fn result_and_length_validity_are_not_silently_retyped() {
    for restricted in 0..2 {
        let mut f = Fixture::new(b"same", b"same", 4);
        let [_, control, effect, result] = f.call(true);
        let ty = if restricted == 0 {
            f.integer
        } else {
            f.graph.rust_type(f.length).unwrap()
        };
        let bits = if restricted == 0 { 32 } else { 64 };
        f.graph.oracle.record_scalar_validity(
            ty,
            ScalarValidity::Scalar(ScalarValidRange::new(bits, 1, 17).unwrap()),
        );
        let compared = f.compare(result, ComparePredicate::Equal, 0, false);
        f.finish(control, effect, compared);
        f.reject();
    }
}
