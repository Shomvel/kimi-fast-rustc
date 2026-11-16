//! One owned enum Phi keeps independent byte lanes until its later complete
//! TransmuteValue licenses whole-word observation.
use crate::{
    CallTarget, ComparePredicate, FunctionGraph, GraphLevel, LayoutOptions, LayoutStats, NodeId,
    Opcode, ValueType, lower_layout_with_options, verify_graph,
};
use kfrc_body::{
    AggregateLayout, EnumLayout, FieldLayout, FunctionAbi, IntegerWidth, Mutability, PassMode,
    RecordedTypes, SizeAlign, TagEncoding, TyId, TypeOracle, TypeShape,
};

pub const SENTINEL: u64 = 0xbad0_dead_face_f00d;

#[derive(Clone, Copy)]
pub struct Options {
    pub bytes: u64,
    pub reverse: bool,
    pub overwrite: bool,
    pub byte_swap: bool,
}

pub fn graph(name: &str, options: Options) -> FunctionGraph {
    let mut types = RecordedTypes::new();
    let byte = integer(&mut types, 1);
    let word = integer(&mut types, options.bytes);
    let address_word = integer(&mut types, 8);
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let array = types.intern(
        TypeShape::Array {
            element: byte,
            length: options.bytes,
        },
        Some(SizeAlign {
            size: options.bytes,
            align: 1,
        }),
    );
    let pointer = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: array,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let result = types.intern(
        TypeShape::Adt {
            name: format!("OwnedBytes{}", options.bytes),
        },
        Some(SizeAlign {
            size: options.bytes + 1,
            align: 1,
        }),
    );
    types.record_aggregate(
        result,
        AggregateLayout {
            fields: vec![],
            variants: vec![
                vec![FieldLayout {
                    ty: array,
                    offset: 1,
                }],
                vec![],
            ],
        },
    );
    types.record_semantic_enum(result, vec![0, 1]);
    types.enums.insert(
        result,
        EnumLayout {
            tag: Some(FieldLayout {
                ty: byte,
                offset: 0,
            }),
            encoding: TagEncoding::Direct {
                discriminants: vec![0, 1],
            },
        },
    );
    let mut graph = FunctionGraph::new(
        name,
        FunctionAbi {
            args: vec![PassMode::Direct; 2],
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
            ValueType::Rust(pointer),
            ValueType::Rust(address_word),
        ]),
    );
    graph.start = start;
    let mut control = project(&mut graph, start, 0, ValueType::Control);
    let mut effect = project(&mut graph, start, 1, ValueType::Effect);
    let source = project(&mut graph, start, 2, ValueType::Rust(pointer));
    let length = project(&mut graph, start, 3, ValueType::Rust(address_word));
    probe(&mut graph, &mut control, &mut effect, address_word, 1);
    let required = graph.add_constant(u128::from(options.bytes), address_word);
    let condition = graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[length, required],
        ComparePredicate::UnsignedGreaterEqual as u32,
        ValueType::Rust(boolean),
    );
    let branch = graph.add_node(
        Opcode::If,
        control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let yes = project(&mut graph, branch, 0, ValueType::Control);
    let no = project(&mut graph, branch, 1, ValueType::Control);
    let before_effect = effect;
    control = yes;
    let snapshot = fixed(
        &mut graph,
        &mut control,
        &mut effect,
        Opcode::LoadValue,
        &[source],
        Some(array),
    )
    .unwrap();
    let yes_value = graph.add_node(
        Opcode::MakeVariant,
        NodeId::NONE,
        &[snapshot],
        0,
        ValueType::Rust(result),
    );
    let no_value = graph.add_node(
        Opcode::MakeVariant,
        NodeId::NONE,
        &[],
        1,
        ValueType::Rust(result),
    );
    let yes = graph.add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
    let no = graph.add_node(Opcode::Jump, no, &[], 0, ValueType::Control);
    let (edges, values, effects) = if options.reverse {
        ([no, yes], [no_value, yes_value], [before_effect, effect])
    } else {
        ([yes, no], [yes_value, no_value], [effect, before_effect])
    };
    let region = graph.add_node(Opcode::Region, NodeId::NONE, &edges, 0, ValueType::Control);
    effect = graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, effects[0], effects[1]],
        0,
        ValueType::Effect,
    );
    let joined = graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, values[0], values[1]],
        0,
        ValueType::Rust(result),
    );
    graph.variant_switches.push(vec![0]);
    let switch = graph.add_node(
        Opcode::VariantSwitch,
        region,
        &[joined],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let yes = project(&mut graph, switch, 0, ValueType::Control);
    let no = project(&mut graph, switch, 1, ValueType::Control);
    let joined_effect = effect;
    control = yes;
    let witness = graph.add_node(
        Opcode::VariantWitness,
        yes,
        &[joined, yes],
        0,
        ValueType::Witness,
    );
    let field_index = graph.intern_variant_field(0, 0);
    let payload = graph.add_node(
        Opcode::VariantField,
        NodeId::NONE,
        &[joined, witness],
        field_index,
        ValueType::Rust(array),
    );
    if options.overwrite {
        // This write is after the enum Phi and its original snapshot read. It
        // must not move before the new wide read, or be observed by its value.
        let byte_pointer = graph.oracle.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Mut,
                pointee: byte,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let first = graph.add_node(
            Opcode::PointerCast,
            NodeId::NONE,
            &[source],
            0,
            ValueType::Rust(byte_pointer),
        );
        let zero = graph.add_constant(0, byte);
        fixed(
            &mut graph,
            &mut control,
            &mut effect,
            Opcode::StoreValue,
            &[first, zero],
            None,
        );
    }
    let packed = fixed(
        &mut graph,
        &mut control,
        &mut effect,
        Opcode::TransmuteValue,
        &[payload],
        Some(word),
    )
    .unwrap();
    let packed = if options.byte_swap {
        graph.add_node(
            Opcode::ByteSwap,
            NodeId::NONE,
            &[packed],
            0,
            ValueType::Rust(word),
        )
    } else {
        packed
    };
    let returned = graph.add_node(
        Opcode::IntCast,
        NodeId::NONE,
        &[packed],
        0,
        ValueType::Rust(address_word),
    );
    probe(&mut graph, &mut control, &mut effect, address_word, 2);
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, returned],
        0,
        ValueType::Control,
    );
    control = no;
    effect = joined_effect;
    probe(&mut graph, &mut control, &mut effect, address_word, 2);
    let sentinel = graph.add_constant(u128::from(SENTINEL), address_word);
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, sentinel],
        0,
        ValueType::Control,
    );
    verify_graph(&mut graph).unwrap();
    graph
}

pub fn lower(source: &FunctionGraph, budget: u64) -> (FunctionGraph, LayoutStats) {
    let mut graph = source.clone();
    let stats = lower_layout_with_options(
        &mut graph,
        LayoutOptions {
            max_unrolled_memory_bytes: budget,
        },
    )
    .unwrap();
    verify_graph(&mut graph).unwrap();
    (graph, stats)
}

pub fn expected(bytes: &[u8], swap: bool) -> u64 {
    if swap {
        bytes
            .iter()
            .fold(0, |value, &byte| (value << 8) | u64::from(byte))
    } else {
        bytes.iter().enumerate().fold(0, |value, (offset, &byte)| {
            value | (u64::from(byte) << (offset * 8))
        })
    }
}

pub fn load_sizes(graph: &FunctionGraph) -> Vec<u64> {
    graph
        .ids()
        .filter_map(|node| {
            if graph.op(node) != Opcode::Load {
                return None;
            }
            let ValueType::Tuple(outputs) = graph.value_type(node) else {
                return None;
            };
            let ValueType::Rust(ty) = outputs[2] else {
                return None;
            };
            Some(graph.oracle.size_align(ty).unwrap().size)
        })
        .collect()
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

fn project(graph: &mut FunctionGraph, source: NodeId, slot: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[source], slot, ty)
}

fn fixed(
    graph: &mut FunctionGraph,
    control: &mut NodeId,
    effect: &mut NodeId,
    op: Opcode,
    operands: &[NodeId],
    result: Option<TyId>,
) -> Option<NodeId> {
    let mut inputs = vec![*effect];
    inputs.extend_from_slice(operands);
    let mut outputs = vec![ValueType::Control, ValueType::Effect];
    outputs.extend(result.map(ValueType::Rust));
    let fixed = graph.add_node(op, *control, &inputs, 0, ValueType::Tuple(outputs));
    *control = project(graph, fixed, 0, ValueType::Control);
    *effect = project(graph, fixed, 1, ValueType::Effect);
    result.map(|ty| project(graph, fixed, 2, ValueType::Rust(ty)))
}

fn probe(
    graph: &mut FunctionGraph,
    control: &mut NodeId,
    effect: &mut NodeId,
    word: TyId,
    event: u128,
) {
    let callee = graph.add_callee(CallTarget {
        symbol: "whole_pack_probe".into(),
        abi: FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
    });
    let event = graph.add_constant(event, word);
    let call = graph.add_node(
        Opcode::Call,
        *control,
        &[*effect, event],
        callee,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(word),
        ]),
    );
    *control = project(graph, call, 0, ValueType::Control);
    *effect = project(graph, call, 1, ValueType::Effect);
}
