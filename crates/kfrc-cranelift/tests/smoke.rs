//! Hand-built graphs through the whole machine layer: export, Cranelift
//! verification, compilation and the encoded hand-over.
use cranelift_codegen::ir;
use kfrc_body::{
    CodeRelocationKind, FunctionAbi, IntegerWidth, Mutability, PassMode, RecordedTypes, SizeAlign,
    TyId, TypeShape,
};
use kfrc_cranelift::{Machine, MachineOptions, Optimization, export_function, lower_function};
use kfrc_son::lower_layout::LayoutOptions;
use kfrc_son::{
    ArithmeticFlags, CallTarget, ComparePredicate, FunctionGraph, GraphLevel, NodeId, Opcode,
    ValueType, lower_layout, verify_graph,
};

fn machine(optimization: Optimization) -> Machine {
    Machine::new(MachineOptions {
        optimization,
        verify: true,
        ..MachineOptions::default()
    })
    .unwrap()
}

fn clif(graph: &FunctionGraph) -> String {
    let mut function = ir::Function::new();
    export_function(
        graph,
        &machine(Optimization::Speed),
        LayoutOptions::default(),
        &mut function,
    )
    .unwrap();
    function.display().to_string()
}

fn projection(graph: &mut FunctionGraph, source: NodeId, index: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[source], index, ty)
}

fn branch(graph: &mut FunctionGraph, control: NodeId, condition: NodeId) -> (NodeId, NodeId) {
    let branch = graph.add_node(
        Opcode::If,
        control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
    );
    (
        projection(graph, branch, 0, ValueType::Control),
        projection(graph, branch, 1, ValueType::Control),
    )
}

struct Binary {
    graph: FunctionGraph,
    ty: TyId,
    boolean: TyId,
    control: NodeId,
    effect: NodeId,
    a: NodeId,
    b: NodeId,
}

/// `fn(a: T, b: T) -> T` for an integer `T`, up to its parameters.
fn binary(symbol: &str, signed: bool, width: IntegerWidth) -> Binary {
    let mut types = RecordedTypes::new();
    let bytes = u64::from(width.bits(64) / 8);
    let ty = types.intern(
        TypeShape::Int { width, signed },
        Some(SizeAlign {
            size: bytes,
            align: bytes,
        }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let mut graph = FunctionGraph::new(
        symbol,
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
            ValueType::Rust(ty),
            ValueType::Rust(ty),
        ]),
    );
    graph.start = start;
    let control = projection(&mut graph, start, 0, ValueType::Control);
    let effect = projection(&mut graph, start, 1, ValueType::Effect);
    let a = projection(&mut graph, start, 2, ValueType::Rust(ty));
    let b = projection(&mut graph, start, 3, ValueType::Rust(ty));
    Binary {
        graph,
        ty,
        boolean,
        control,
        effect,
        a,
        b,
    }
}

fn to_layout(mut graph: FunctionGraph) -> FunctionGraph {
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    lower_layout(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    graph
}

/// `acc = b; for i in 0..a { acc = acc.checked_mul(10)? }`, returning 37 on
/// overflow: a region with two phis, a checked product and its reuse.
fn checked_product_loop(signed: bool, width: IntegerWidth) -> FunctionGraph {
    let Binary {
        graph: mut g,
        ty,
        boolean,
        control,
        effect,
        a,
        b,
    } = binary("mul_loop", signed, width);
    let zero = g.add_constant(0, ty);
    let one = g.add_constant(1, ty);
    let entry = g.add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
    let region = g.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry, entry],
        0,
        ValueType::Control,
    );
    let index = g.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, zero, zero],
        0,
        ValueType::Rust(ty),
    );
    let accumulator = g.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, b, b],
        0,
        ValueType::Rust(ty),
    );
    let more = g.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[index, a],
        if signed {
            ComparePredicate::SignedLess
        } else {
            ComparePredicate::UnsignedLess
        } as u32,
        ValueType::Rust(boolean),
    );
    let (body, exit) = branch(&mut g, region, more);
    g.add_node(
        Opcode::Return,
        exit,
        &[effect, accumulator],
        0,
        ValueType::Control,
    );
    let ten = g.add_constant(10, ty);
    let overflow = g.add_node(
        Opcode::MulOverflow,
        body,
        &[accumulator, ten],
        0,
        ValueType::Rust(boolean),
    );
    let (error, success) = branch(&mut g, body, overflow);
    let sentinel = g.add_constant(37, ty);
    g.add_node(
        Opcode::Return,
        error,
        &[effect, sentinel],
        0,
        ValueType::Control,
    );
    let product = g.add_node(
        Opcode::Mul,
        success,
        &[accumulator, ten],
        if signed {
            ArithmeticFlags::NSW
        } else {
            ArithmeticFlags::NUW
        },
        ValueType::Rust(ty),
    );
    let next = g.add_node(Opcode::Add, success, &[index, one], 0, ValueType::Rust(ty));
    let backedge = g.add_node(Opcode::Jump, success, &[], 0, ValueType::Control);
    g.set_input(region, 1, backedge);
    g.set_input(index, 2, next);
    g.set_input(accumulator, 2, product);
    to_layout(g)
}

/// `memmove(dst, src, size)` returning the call's pointer result.
fn copy(symbol: &str, size: u64) -> FunctionGraph {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let pointer = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: word,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let mut graph = FunctionGraph::new(
        "small_copies",
        FunctionAbi {
            args: vec![PassMode::Direct, PassMode::Direct],
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
            ValueType::Rust(pointer),
            ValueType::Rust(pointer),
        ]),
    );
    graph.start = start;
    let control = projection(&mut graph, start, 0, ValueType::Control);
    let effect = projection(&mut graph, start, 1, ValueType::Effect);
    let destination = projection(&mut graph, start, 2, ValueType::Rust(pointer));
    let source = projection(&mut graph, start, 3, ValueType::Rust(pointer));
    let count = graph.add_constant(u128::from(size), word);
    let callee = graph.add_callee(CallTarget {
        symbol: symbol.into(),
        abi: FunctionAbi {
            args: vec![PassMode::Direct; 3],
            ret: PassMode::Direct,
            c_variadic: false,
        },
    });
    let call = graph.add_node(
        Opcode::Call,
        control,
        &[effect, destination, source, count],
        callee,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(pointer),
        ]),
    );
    let control = projection(&mut graph, call, 0, ValueType::Control);
    let effect = projection(&mut graph, call, 1, ValueType::Effect);
    let result = projection(&mut graph, call, 2, ValueType::Rust(pointer));
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, result],
        0,
        ValueType::Control,
    );
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    graph
}

#[test]
fn checked_product_loops_compile_at_every_width_and_level() {
    for optimization in [
        Optimization::None,
        Optimization::Speed,
        Optimization::SpeedAndSize,
    ] {
        for signed in [false, true] {
            for width in [
                IntegerWidth::W8,
                IntegerWidth::W16,
                IntegerWidth::W32,
                IntegerWidth::W64,
            ] {
                let graph = checked_product_loop(signed, width);
                let function = lower_function(&graph, &machine(optimization))
                    .unwrap_or_else(|error| panic!("{error}\n{}", clif(&graph)));
                assert_eq!(function.symbol, "mul_loop");
                assert!(!function.bytes.is_empty());
                assert!(function.align >= 16);
                assert!(function.relocations.is_empty());
            }
        }
    }
}

#[test]
fn one_multiply_serves_the_overflow_check_and_the_product() {
    let text = clif(&checked_product_loop(false, IntegerWidth::W64));
    assert_eq!(text.matches("umul_overflow").count(), 1, "{text}");
    assert_eq!(text.matches("imul").count(), 0, "{text}");
}

#[test]
fn small_copies_become_loads_and_stores() {
    for symbol in ["memmove", "memcpy"] {
        for (size, loads) in [(8u64, 1usize), (15, 2), (31, 2), (32, 2), (64, 4)] {
            let graph = copy(symbol, size);
            let text = clif(&graph);
            assert_eq!(text.matches(" = load.").count(), loads, "{symbol} {size}\n{text}");
            assert_eq!(text.matches("store ").count(), loads, "{symbol} {size}\n{text}");
            let function = lower_function(&graph, &machine(Optimization::Speed)).unwrap();
            assert!(function.relocations.is_empty(), "{symbol} {size}");
        }
    }
}

#[test]
fn larger_copies_stay_library_calls() {
    let function = lower_function(&copy("memmove", 65), &machine(Optimization::Speed)).unwrap();
    let [relocation] = function.relocations.as_slice() else {
        panic!("{:?}", function.relocations);
    };
    assert_eq!(relocation.target, "memmove");
    assert_eq!(relocation.kind, CodeRelocationKind::Call);
    assert_eq!(relocation.addend, -4);
}
