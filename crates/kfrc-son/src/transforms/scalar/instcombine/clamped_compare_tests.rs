use super::*;
use crate::{
    ComparePredicate as P, GraphLevel, InterpretError, Interpreter, Opcode, Outcome,
    SemanticOutcome, Value, ValueType, lower_layout, verify_graph,
};
use kfrc_body::{
    DataObject, FunctionAbi, IntegerWidth as W, Mutability, PassMode, RecordedTypes,
    ScalarValidRange, ScalarValidity, SizeAlign, TyId, TypeShape,
};

struct Fixture {
    graph: FunctionGraph,
    source_ty: TyId,
    word: TyId,
    signed_word: TyId,
    boolean: TyId,
    args: [NodeId; 2],
    control: NodeId,
    effect: NodeId,
}

struct Decoder {
    selected: NodeId,
    range: NodeId,
    mask: NodeId,
    extended: NodeId,
    yes: NodeId,
    no: NodeId,
    inverse: NodeId,
}

impl Fixture {
    fn new(source: W, target: W) -> Self {
        let mut types = RecordedTypes::new();
        let mut int = |width: W, signed| {
            let bytes = u64::from(width.bits(64) / 8);
            types.intern(
                TypeShape::Int { width, signed },
                Some(SizeAlign {
                    size: bytes,
                    align: bytes,
                }),
            )
        };
        let source_ty = int(source, false);
        let word = int(target, false);
        let signed_word = int(target, true);
        let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let mut graph = FunctionGraph::new(
            "clamped_compare",
            FunctionAbi {
                args: vec![PassMode::Direct; 2],
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
                ValueType::Rust(source_ty),
                ValueType::Rust(source_ty),
            ]),
        );
        let mut projection =
            |slot, ty| graph.add_node(Opcode::Projection, NodeId::NONE, &[graph.start], slot, ty);
        let control = projection(0, ValueType::Control);
        let effect = projection(1, ValueType::Effect);
        let args = [
            projection(2, ValueType::Rust(source_ty)),
            projection(3, ValueType::Rust(source_ty)),
        ];
        Self {
            graph,
            source_ty,
            word,
            signed_word,
            boolean,
            args,
            control,
            effect,
        }
    }

    fn scalar(&mut self, op: Opcode, inputs: &[NodeId], payload: u32, ty: TyId) -> NodeId {
        self.graph
            .add_node(op, NodeId::NONE, inputs, payload, ValueType::Rust(ty))
    }

    fn cast(&mut self, value: NodeId, ty: TyId) -> NodeId {
        self.scalar(Opcode::IntCast, &[value], 0, ty)
    }

    fn decoder(&mut self, source: NodeId, limit: u64, fallback: u64, order: u8) -> Decoder {
        let limit = self.graph.add_constant(u128::from(limit), self.source_ty);
        let range = self.scalar(
            Opcode::Compare,
            &[source, limit],
            P::UnsignedLessEqual as u32,
            self.boolean,
        );
        let flag = self.cast(range, self.word);
        let mask = self.scalar(Opcode::Negate, &[flag], 0, self.word);
        let extended = if self.source_ty == self.word {
            source
        } else {
            self.cast(source, self.word)
        };
        let inverse = self.scalar(Opcode::BitNot, &[mask], 0, self.word);
        let fallback = self.graph.add_constant(u128::from(fallback), self.word);
        let yes_inputs = if order & 1 == 0 {
            [mask, extended]
        } else {
            [extended, mask]
        };
        let no_inputs = if order & 2 == 0 {
            [inverse, fallback]
        } else {
            [fallback, inverse]
        };
        let yes = self.scalar(Opcode::BitAnd, &yes_inputs, 0, self.word);
        let no = self.scalar(Opcode::BitAnd, &no_inputs, 0, self.word);
        let terms = if order & 4 == 0 { [yes, no] } else { [no, yes] };
        let selected = self.scalar(Opcode::BitOr, &terms, 0, self.word);
        Decoder {
            selected,
            range,
            mask,
            extended,
            yes,
            no,
            inverse,
        }
    }

    fn comparison(
        &mut self,
        value: NodeId,
        expected: u64,
        predicate: P,
        swapped: bool,
        casts: usize,
    ) -> NodeId {
        let mut value = value;
        for cast in 0..casts {
            value = self.cast(
                value,
                if cast % 2 == 0 {
                    self.signed_word
                } else {
                    self.word
                },
            );
        }
        let ty = self.graph.rust_type(value).unwrap();
        let expected = self.graph.add_constant(u128::from(expected), ty);
        let inputs = if swapped {
            [expected, value]
        } else {
            [value, expected]
        };
        self.scalar(Opcode::Compare, &inputs, predicate as u32, self.boolean)
    }

    fn finish(&mut self, value: NodeId) {
        self.graph.add_node(
            Opcode::Return,
            self.control,
            &[self.effect, value],
            0,
            ValueType::Control,
        );
        verify_graph(&mut self.graph).unwrap();
    }

    fn rewrite(&mut self, comparison: NodeId) -> FunctionGraph {
        let before = self.graph.clone();
        let len = self.graph.len();
        let replacement = combine(&mut self.graph, comparison);
        assert_eq!(replacement.is_some(), enabled());
        assert!(self.graph.len() - len <= 2);
        if let Some(replacement) = replacement {
            self.graph.replace(comparison, replacement);
            self.graph.resolve_edges();
        }
        verify_graph(&mut self.graph).unwrap();
        before
    }

    fn reject(&mut self, comparison: NodeId) {
        let before = self.graph.dump();
        assert!(combine(&mut self.graph, comparison).is_none());
        assert_eq!(
            self.graph.dump(),
            before,
            "a failed match must not allocate or edit"
        );
    }
}

fn run(graph: &FunctionGraph, value: u64) -> Result<Outcome, InterpretError> {
    Interpreter::new(graph).run(
        &[value, value ^ 1],
        &mut |_: &str, _: &[u64]| unreachable!(),
    )
}

#[test]
fn exhaustive_bytes_cover_ranges_fallbacks_equality_order_and_signed_bit_casts() {
    for limit in [0, 1, 4, 127, 254, 255] {
        for fallback in [limit + 1, 65535] {
            for expected in [0, limit, fallback] {
                for predicate in [P::Equal, P::NotEqual] {
                    for order in 0..8 {
                        for casts in 0..=2 {
                            let mut f = Fixture::new(W::W8, W::W16);
                            let decoded = f.decoder(f.args[0], limit, fallback, order);
                            let cmp = f.comparison(
                                decoded.selected,
                                expected,
                                predicate,
                                order & 1 != 0,
                                casts,
                            );
                            f.finish(cmp);
                            let before = f.rewrite(cmp);
                            for x in 0..256 {
                                let selected = if x <= limit { x } else { fallback };
                                let equal = selected == expected;
                                let result = u64::from(equal ^ (predicate == P::NotEqual));
                                assert_eq!(run(&before, x), Ok(Outcome::Return(result)));
                                assert_eq!(
                                    run(&f.graph, x),
                                    Ok(Outcome::Return(result)),
                                    "x={x}, K={limit}, C={fallback}, E={expected}, {predicate:?}, order={order}, casts={casts}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn wider_zero_extensions_and_source_boundaries_match_the_decoder() {
    for (source, target) in [
        (W::W8, W::W8),
        (W::W8, W::W64),
        (W::W16, W::W32),
        (W::W32, W::W64),
        (W::W64, W::W64),
        (W::Pointer, W::Pointer),
    ] {
        let max = u64::MAX >> (64 - source.bits(64));
        for limit in [0, 4, max - 1] {
            for expected in [0, limit, max] {
                for predicate in [P::Equal, P::NotEqual] {
                    let mut f = Fixture::new(source, target);
                    let decoded = f.decoder(f.args[0], limit, max, 0);
                    let cmp = f.comparison(decoded.selected, expected, predicate, false, 1);
                    f.finish(cmp);
                    let before = f.rewrite(cmp);
                    for x in [0, 1, 3, 4, 5, limit, limit + 1, max - 1, max] {
                        assert_eq!(run(&before, x), run(&f.graph, x));
                    }
                }
            }
        }
    }
}

#[test]
fn shared_decoded_value_survives_and_both_replacement_cases_observe_source_poison() {
    for expected in [2, 5] {
        let mut f = Fixture::new(W::W8, W::W64);
        let decoded = f.decoder(f.args[0], 4, 5, 0);
        let cmp = f.comparison(decoded.selected, expected, P::Equal, false, 1);
        let flag = f.cast(cmp, f.word);
        let result = f.scalar(Opcode::Add, &[decoded.selected, flag], 0, f.word);
        f.finish(result);
        let before = f.rewrite(cmp);
        for x in 0..256 {
            assert_eq!(run(&before, x), run(&f.graph, x));
        }
        f.graph.compact();
        assert!(f.graph.ids().any(|n| f.graph.op(n) == Opcode::BitOr));

        let mut f = Fixture::new(W::W8, W::W64);
        let poison = f.scalar(Opcode::Undefined, &[], 0, f.source_ty);
        let decoded = f.decoder(poison, 4, 5, 0);
        let cmp = f.comparison(decoded.selected, expected, P::NotEqual, true, 1);
        f.finish(cmp);
        let before = f.rewrite(cmp);
        assert!(matches!(
            run(&before, 0),
            Err(InterpretError::PoisonObserved(_))
        ));
        assert!(matches!(
            run(&f.graph, 0),
            Err(InterpretError::PoisonObserved(_))
        ));
    }
}

#[test]
fn bounded_seeded_cleanup_rewrites_the_same_comparison() {
    let mut f = Fixture::new(W::W8, W::W64);
    let decoded = f.decoder(f.args[0], 4, 5, 0);
    let cmp = f.comparison(decoded.selected, 2, P::Equal, false, 1);
    f.finish(cmp);
    let before = f.graph.clone();
    let mut work = crate::transforms::scalar::seam::Worklist::new(&f.graph);
    let stats = work.run_bounded(&mut f.graph, &[cmp], 2);
    assert_eq!(f.graph.resolve(cmp) != cmp, enabled());
    if enabled() {
        assert!(stats.identities_simplified > 0);
    }
    f.graph.resolve_edges();
    verify_graph(&mut f.graph).unwrap();
    for x in 0..256 {
        assert_eq!(run(&before, x), run(&f.graph, x));
    }
}

#[test]
fn mismatched_sources_masks_contracts_flags_widths_and_nonconstant_comparisons_are_rejected() {
    for case in 0..24 {
        let mut f = Fixture::new(W::W8, W::W64);
        let decoded = f.decoder(f.args[0], 4, 5, 0);
        let cmp = f.comparison(decoded.selected, 2, P::Equal, false, 1);
        match case {
            0 => f.graph.set_input(decoded.extended, 0, f.args[1]),
            1 => {
                let other = f.scalar(Opcode::Negate, &[f.args[1]], 0, f.word);
                f.graph.set_input(decoded.inverse, 0, other);
            }
            2 => {
                let fallback = f.graph.add_constant(4, f.word);
                f.graph.set_input(decoded.no, 1, fallback);
            }
            3 => {
                let expected = f.graph.add_constant(7, f.signed_word);
                f.graph.set_input(cmp, 1, expected);
            }
            4 => {
                let expected = f.cast(f.args[1], f.signed_word);
                f.graph.set_input(cmp, 1, expected);
            }
            5 => f.graph.set_payload(decoded.range, P::UnsignedLess as u32),
            6 => f.graph.set_payload(cmp, P::UnsignedLessEqual as u32),
            7 => f.graph.level = GraphLevel::Specialized,
            8 => f.graph.oracle.record_scalar_validity(
                f.source_ty,
                ScalarValidity::Scalar(ScalarValidRange::new(8, 1, 255).unwrap()),
            ),
            9 => f.graph.oracle.record_scalar_validity(
                f.word,
                ScalarValidity::Scalar(ScalarValidRange::new(64, 1, u64::MAX as u128).unwrap()),
            ),
            10 => {
                let ty = f.graph.oracle.intern(
                    TypeShape::Int {
                        width: W::W32,
                        signed: false,
                    },
                    None,
                );
                let value = f.cast(decoded.selected, ty);
                let expected = f.graph.add_constant(2, ty);
                f.graph.set_inputs(cmp, &[value, expected]);
            }
            11 => {
                let ty = f.graph.oracle.intern(
                    TypeShape::Int {
                        width: W::W8,
                        signed: true,
                    },
                    None,
                );
                f.graph.set_value_type(f.args[0], ValueType::Rust(ty));
            }
            12..=17 => {
                let n = [
                    decoded.selected,
                    decoded.yes,
                    decoded.no,
                    decoded.mask,
                    decoded.extended,
                    decoded.inverse,
                ][case - 12];
                f.graph.set_payload(n, 1);
            }
            _ => {
                let n = [
                    cmp,
                    decoded.selected,
                    decoded.range,
                    decoded.mask,
                    decoded.extended,
                    decoded.inverse,
                ][case - 18];
                f.graph.set_control(n, f.control);
            }
        }
        // Several cases intentionally have invalid flags/control/type edges.
        // Test only the read-only matcher boundary, not the verifier/interpreter.
        f.reject(cmp);
    }
    for (source, target) in [(W::W8, W::W128), (W::W128, W::W128), (W::W64, W::W32)] {
        let mut f = Fixture::new(source, target);
        let decoded = f.decoder(f.args[0], 4, 5, 0);
        let cmp = f.comparison(decoded.selected, 2, P::Equal, false, 1);
        f.reject(cmp);
    }
    for case in 0..5 {
        let mut f = Fixture::new(W::W8, W::W64);
        let decoded = f.decoder(f.args[0], 4, 5, 0);
        let cmp = f.comparison(
            decoded.selected,
            2,
            P::Equal,
            false,
            if case == 0 { 3 } else { 1 },
        );
        match case {
            0 => {} // Match depth is bounded even for a longer bit-preserving cast chain.
            1 => f
                .graph
                .set_value_type(decoded.selected, ValueType::Rust(f.signed_word)),
            2 => {
                let flag = f.graph.input(decoded.mask, 0);
                f.graph.set_payload(flag, 1);
            }
            3 => f.graph.set_control(decoded.yes, f.control),
            _ => f.graph.set_control(decoded.no, f.control),
        }
        f.reject(cmp);
    }
}

#[test]
fn layout_pipeline_folds_a_static_decoded_rhs_before_simplifying_the_dynamic_compare() {
    let mut f = Fixture::new(W::W8, W::W64);
    let pointer = f.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: f.source_ty,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    f.graph.symbols.push(("tag".into(), 0));
    f.graph.data_objects.push(DataObject {
        symbol: "tag".into(),
        bytes: vec![4],
        align: 1,
        ..Default::default()
    });
    let address = f.scalar(Opcode::SymbolAddress, &[], 0, pointer);
    let load = f.graph.add_node(
        Opcode::LoadValue,
        f.control,
        &[f.effect, address],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(f.source_ty),
        ]),
    );
    f.control = f.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[load],
        0,
        ValueType::Control,
    );
    f.effect = f.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[load],
        1,
        ValueType::Effect,
    );
    let tag = f.scalar(Opcode::Projection, &[load], 2, f.source_ty);
    let two = f.graph.add_constant(2, f.source_ty);
    let x = f.scalar(Opcode::Sub, &[f.args[0], two], 0, f.source_ty);
    let y = f.scalar(Opcode::Sub, &[tag, two], 0, f.source_ty);
    let left = f.decoder(x, 4, 5, 0);
    let right = f.decoder(y, 4, 5, 0);
    let a = f.cast(left.selected, f.signed_word);
    let b = f.cast(right.selected, f.signed_word);
    let cmp = f.scalar(Opcode::Compare, &[a, b], P::Equal as u32, f.boolean);
    f.graph.level = GraphLevel::Specialized;
    f.finish(cmp);
    let before = f.graph.clone();
    lower_layout(&mut f.graph).unwrap();
    verify_graph(&mut f.graph).unwrap();
    assert!(!f.graph.ids().any(|n| f.graph.op(n) == Opcode::Load));
    assert_eq!(
        f.graph.ids().any(|n| f.graph.op(n) == Opcode::Negate),
        !enabled()
    );
    for raw in 0..256 {
        let expected = u64::from(raw == 4);
        assert_eq!(
            Interpreter::new(&before).run_values(
                &[Value::Scalar(raw), Value::Scalar(0)],
                &mut |_: &str, _: &[Value]| unreachable!()
            ),
            Ok(SemanticOutcome::Return(Value::Scalar(expected)))
        );
        assert_eq!(run(&f.graph, raw), Ok(Outcome::Return(expected)));
    }
}
