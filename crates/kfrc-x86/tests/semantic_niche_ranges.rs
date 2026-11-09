//! Niche tags are compared against their legal encodings. rustc's niche
//! range may include the untagged variant; that relative slot is never a
//! valid tag, so every valid tag still selects exactly one variant.
use kfrc_body::*;
use kfrc_son::*;
use kfrc_x86::lower_function;

#[derive(Clone)]
struct Encoding {
    name: &'static str,
    untagged_variant: u32,
    niche_variants: std::ops::RangeInclusive<u32>,
    niche_start: u128,
    /// Every valid tag byte and the variant it encodes.
    valid_tags: Vec<(u64, u64)>,
}

fn encodings() -> Vec<Encoding> {
    // The untagged variant carries a `bool` in the tag byte: tags 0 and 1.
    vec![
        // `enum { A, B(bool), C }`: B is untagged inside the niche range 0..=2.
        Encoding {
            name: "untagged_inside_niche_range",
            untagged_variant: 1,
            niche_variants: 0..=2,
            niche_start: 2,
            valid_tags: vec![(0, 1), (1, 1), (2, 0), (4, 2)],
        },
        // `Option<bool>`: the single niche variant precedes the untagged one.
        Encoding {
            name: "singleton_before_untagged",
            untagged_variant: 1,
            niche_variants: 0..=0,
            niche_start: 2,
            valid_tags: vec![(0, 1), (1, 1), (2, 0)],
        },
        // `enum { A(bool), B }`: the single niche variant follows it.
        Encoding {
            name: "singleton_after_untagged",
            untagged_variant: 0,
            niche_variants: 1..=1,
            niche_start: 2,
            valid_tags: vec![(0, 0), (1, 0), (2, 1)],
        },
        // A byte niche that wraps past 255; the untagged payload's valid
        // range is 2..=3 here, so the wrapped tag 0 is free.
        Encoding {
            name: "wrapping_niche_after_untagged",
            untagged_variant: 0,
            niche_variants: 1..=3,
            niche_start: 254,
            valid_tags: vec![(2, 0), (3, 0), (254, 1), (255, 2), (0, 3)],
        },
        // `enum { A, B, C(bool) }`: untagged last, niche range before it.
        Encoding {
            name: "niche_range_before_untagged",
            untagged_variant: 2,
            niche_variants: 0..=1,
            niche_start: 2,
            valid_tags: vec![(0, 2), (1, 2), (2, 0), (3, 1)],
        },
    ]
}

#[derive(Clone, Copy)]
enum Query {
    VariantOf,
    Discriminant,
    Switch,
    /// Every variant is a case; the remaining edge is `Unreachable`.
    ExhaustiveSwitch,
}

const DISCRIMINANTS: [u128; 4] = [5, 7, 9, 11];

fn graph(encoding: &Encoding, query: Query) -> FunctionGraph {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let byte = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: false,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let enumeration = types.intern(
        TypeShape::Adt {
            name: encoding.name.into(),
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let count = 1 + *encoding
        .niche_variants
        .end()
        .max(&encoding.untagged_variant) as usize;
    let mut variants = vec![vec![]; count];
    variants[encoding.untagged_variant as usize] = vec![FieldLayout {
        ty: boolean,
        offset: 0,
    }];
    types.record_aggregate(
        enumeration,
        AggregateLayout {
            fields: vec![],
            variants,
        },
    );
    types.record_semantic_enum(enumeration, DISCRIMINANTS[..count].to_vec());
    types.enums.insert(
        enumeration,
        EnumLayout {
            tag: Some(FieldLayout {
                ty: byte,
                offset: 0,
            }),
            encoding: TagEncoding::Niche {
                untagged_variant: encoding.untagged_variant,
                niche_variants: encoding.niche_variants.clone(),
                niche_start: encoding.niche_start,
            },
        },
    );
    let mut graph = FunctionGraph::new(
        encoding.name,
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    graph.level = GraphLevel::Specialized;
    let start = graph.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(enumeration),
        ]),
    );
    graph.start = start;
    let projection = |graph: &mut FunctionGraph, index, ty| {
        graph.add_node(Opcode::Projection, NodeId::NONE, &[start], index, ty)
    };
    let control = projection(&mut graph, 0, ValueType::Control);
    let effect = projection(&mut graph, 1, ValueType::Effect);
    let input = projection(&mut graph, 2, ValueType::Rust(enumeration));
    match query {
        Query::VariantOf | Query::Discriminant => {
            let op = match query {
                Query::VariantOf => Opcode::VariantOf,
                _ => Opcode::Discriminant,
            };
            let value = graph.add_node(op, NodeId::NONE, &[input], 0, ValueType::Rust(word));
            graph.add_node(
                Opcode::Return,
                control,
                &[effect, value],
                0,
                ValueType::Control,
            );
        }
        Query::Switch | Query::ExhaustiveSwitch => {
            // Test the untagged variant first, then the niche variants in
            // descending order; the default edge is the lowest variant.
            let mut cases = vec![encoding.untagged_variant];
            cases.extend(
                (1..count as u32)
                    .rev()
                    .filter(|&variant| variant != encoding.untagged_variant),
            );
            if encoding.untagged_variant == 0 {
                cases.retain(|&variant| variant != 1);
            }
            let remaining = (0..count as u32)
                .find(|variant| !cases.contains(variant))
                .unwrap();
            let exhaustive = matches!(query, Query::ExhaustiveSwitch);
            if exhaustive {
                cases.push(remaining);
            }
            let mut targets: Vec<_> = cases.iter().copied().map(Some).collect();
            targets.push((!exhaustive).then_some(remaining));
            graph.variant_switches.push(cases);
            let branch = graph.add_node(
                Opcode::VariantSwitch,
                control,
                &[input],
                0,
                ValueType::Tuple(vec![ValueType::Control; targets.len()]),
            );
            for (edge, variant) in targets.into_iter().enumerate() {
                let control = graph.add_node(
                    Opcode::Projection,
                    NodeId::NONE,
                    &[branch],
                    edge as u32,
                    ValueType::Control,
                );
                let Some(variant) = variant else {
                    graph.add_node(Opcode::Unreachable, control, &[], 0, ValueType::Control);
                    continue;
                };
                let result = graph.add_constant(100 + u128::from(variant), word);
                graph.add_node(
                    Opcode::Return,
                    control,
                    &[effect, result],
                    0,
                    ValueType::Control,
                );
            }
        }
    }
    graph
}

fn expected(query: Query, variant: u64) -> u64 {
    match query {
        Query::VariantOf => variant,
        Query::Discriminant => DISCRIMINANTS[variant as usize] as u64,
        Query::Switch | Query::ExhaustiveSwitch => 100 + variant,
    }
}

fn run_layout(graph: &FunctionGraph, tag: u64) -> u64 {
    match Interpreter::new(graph)
        .run(&[tag], &mut |name: &str, _: &[u64]| {
            Err(InterpretError::UnknownCallee(name.into()))
        })
        .unwrap_or_else(|error| panic!("{error:?}\n{}", graph.dump()))
    {
        Outcome::Return(value) => value,
        other => panic!("unexpected outcome {other:?}"),
    }
}

#[test]
fn every_valid_niche_tag_selects_its_variant_in_layout_and_machine_code() {
    for encoding in encodings() {
        for query in [
            Query::VariantOf,
            Query::Discriminant,
            Query::Switch,
            Query::ExhaustiveSwitch,
        ] {
            let mut graph = graph(&encoding, query);
            verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
            lower_layout(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
            verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
            let machine = lower_function(&graph).unwrap();
            for &(tag, variant) in &encoding.valid_tags {
                let want = expected(query, variant);
                assert_eq!(
                    run_layout(&graph, tag),
                    want,
                    "{} layout tag {tag}\n{}",
                    encoding.name,
                    graph.dump()
                );
                assert_eq!(
                    kfrc_x86::machine::eval::evaluate(&machine, &[tag as i64]).unwrap(),
                    want as i64,
                    "{} machine tag {tag}",
                    encoding.name
                );
            }
        }
    }
}

#[test]
fn singleton_niche_index_is_one_tag_comparison_without_selection_arithmetic() {
    for encoding in encodings()
        .into_iter()
        .filter(|encoding| encoding.niche_variants.start() == encoding.niche_variants.end())
    {
        let mut graph = graph(&encoding, Query::VariantOf);
        verify_graph(&mut graph).unwrap();
        lower_layout(&mut graph).unwrap();
        let count = |op| graph.ids().filter(|&node| graph.op(node) == op).count();
        assert_eq!(count(Opcode::Compare), 1, "{}", graph.dump());
        for op in [
            Opcode::Negate,
            Opcode::BitNot,
            Opcode::BitAnd,
            Opcode::BitOr,
            Opcode::Sub,
        ] {
            assert_eq!(count(op), 0, "{op:?} in\n{}", graph.dump());
        }
    }
}

#[test]
fn exhaustive_switch_selects_its_last_case_without_comparing() {
    for encoding in encodings() {
        let branches = |query| {
            let mut graph = graph(&encoding, query);
            verify_graph(&mut graph).unwrap();
            let stats = lower_layout(&mut graph).unwrap();
            let ifs = graph.ids().filter(|&n| graph.op(n) == Opcode::If).count();
            let unreachable = graph
                .ids()
                .filter(|&n| graph.op(n) == Opcode::Unreachable)
                .count();
            (ifs, unreachable, stats.exhaustive_switch_compares_removed)
        };
        let (open_ifs, open_unreachable, open_removed) = branches(Query::Switch);
        let (closed_ifs, closed_unreachable, closed_removed) = branches(Query::ExhaustiveSwitch);
        // The reachable default edge keeps every case comparison.
        assert_eq!(
            (open_unreachable, open_removed),
            (0, 0),
            "{}",
            encoding.name
        );
        assert_eq!(
            (closed_unreachable, closed_removed),
            (0, 1),
            "{}",
            encoding.name
        );
        assert_eq!(closed_ifs, open_ifs, "{}", encoding.name);
    }
}
