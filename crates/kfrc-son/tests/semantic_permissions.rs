//! RS5 borrow permissions: the S carrier (RS5.1) and Frozen load reuse
//! (RS5.2 Q1–Q3). Graphs come from production Body construction; each
//! optimized graph is compared with its input under the reference interpreter.

use kfrc_body::*;
use kfrc_son::{
    ArgPermission, FunctionGraph, GraphLevel, InterpretError, Interpreter, Opcode, OptimizeStats,
    SemanticCallHandler, SemanticMemory, SemanticOutcome, Value, build_graph,
    optimize_specialized_graph, verify_graph,
};

const FROZEN_WORD: ArgPermission = ArgPermission::Frozen {
    dereferenceable: Some(8),
};

struct Types {
    types: RecordedTypes,
    word: TyId,
    unit: TyId,
    boolean: TyId,
}

fn types() -> Types {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let unit = types.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    Types {
        types,
        word,
        unit,
        boolean,
    }
}

impl Types {
    fn pointer(&mut self, shape: fn(Mutability, TyId) -> TypeShape, mutability: Mutability, pointee: TyId) -> TyId {
        self.types.intern(
            shape(mutability, pointee),
            Some(SizeAlign { size: 8, align: 8 }),
        )
    }

    fn reference(&mut self, mutability: Mutability, pointee: TyId) -> TyId {
        self.pointer(
            |mutability, pointee| TypeShape::Reference {
                mutability,
                pointee,
            },
            mutability,
            pointee,
        )
    }

    fn raw(&mut self, mutability: Mutability, pointee: TyId) -> TyId {
        self.pointer(
            |mutability, pointee| TypeShape::RawPointer {
                mutability,
                pointee,
            },
            mutability,
            pointee,
        )
    }
}

fn local(builder: &mut BodyBuilder, ty: TyId) -> LocalId {
    builder.add_local(LocalDecl {
        ty,
        mutability: Mutability::Mut,
        name: None,
    })
}

fn copy(place: Place) -> Operand {
    Operand::Copy(place)
}

fn deref(builder: &mut BodyBuilder, pointer: LocalId) -> Place {
    Place {
        local: pointer,
        projection: builder.add_projection(vec![ProjectionElem::Deref]),
    }
}

fn assign(builder: &mut BodyBuilder, place: Place, value: Rvalue) {
    builder.push_statement(StatementKind::Assign(place, value));
}

fn scalar(builder: &mut BodyBuilder, ty: TyId, bits: u128) -> Operand {
    Operand::Constant(builder.add_constant(Constant {
        ty,
        kind: ConstantKind::Scalar(bits),
    }))
}

/// `destination = symbol(args)`, continuing at `target`.
fn call(
    builder: &mut BodyBuilder,
    unit: TyId,
    symbol: &str,
    args: Vec<Operand>,
    destination: LocalId,
    target: u32,
) {
    let callee = Operand::Constant(builder.add_constant(Constant {
        ty: unit,
        kind: ConstantKind::Function(Callee::Function {
            symbol: symbol.into(),
            abi: FunctionAbi {
                args: vec![PassMode::Direct; args.len()],
                ret: PassMode::Ignore,
                c_variadic: false,
            },
        }),
    }));
    builder.finish_block(
        Terminator::Call {
            callee,
            args,
            destination: Place::local(destination),
            target: Some(BlockId(target)),
            unwind: UnwindAction::Unreachable,
        },
        false,
    );
}

fn add(builder: &mut BodyBuilder, destination: LocalId, a: LocalId, b: LocalId) {
    assign(
        builder,
        Place::local(destination),
        Rvalue::BinaryOp(
            BinaryOp::Add,
            copy(Place::local(a)),
            copy(Place::local(b)),
        ),
    );
}

fn graph(body: Body, permissions: Vec<ArgPermission>) -> FunctionGraph {
    let mut graph = build_graph(&body).unwrap();
    graph.arg_permissions = permissions;
    verify_graph(&mut graph).unwrap();
    graph
}

fn loads(graph: &FunctionGraph) -> usize {
    graph
        .ids()
        .filter(|&node| graph.op(node) == Opcode::LoadValue)
        .count()
}

/// Calls either leave memory alone or write `value` at `address`.
struct CallEffects {
    write: Option<(u64, u64)>,
    calls: usize,
}

impl SemanticCallHandler for CallEffects {
    fn call(&mut self, _: &str, _: &[Value]) -> Result<Value, InterpretError> {
        unreachable!("calls receive memory")
    }

    fn call_with_memory(
        &mut self,
        _: &str,
        _: &[Value],
        memory: &mut SemanticMemory,
    ) -> Result<Value, InterpretError> {
        self.calls += 1;
        if let Some((address, value)) = self.write {
            let bytes: Vec<_> = value.to_le_bytes().into_iter().map(Some).collect();
            memory.write_bytes(address, &bytes)?;
        }
        Ok(Value::Aggregate(vec![]))
    }
}

/// Runs `graph` with one heap word holding `initial`; argument values may
/// name it through `ADDRESS`. Returns the outcome and the number of calls.
const ADDRESS: u64 = u64::MAX;

fn run(
    graph: &FunctionGraph,
    arguments: &[Value],
    initial: u64,
    write: Option<u64>,
) -> (Result<SemanticOutcome, InterpretError>, usize) {
    let mut memory = SemanticMemory::new();
    let address = memory.allocate(16, 8, true).unwrap();
    let bytes: Vec<_> = initial.to_le_bytes().into_iter().map(Some).collect();
    memory.write_bytes(address, &bytes).unwrap();
    fn substitute(value: &Value, address: u64) -> Value {
        match value {
            Value::Scalar(ADDRESS) => Value::Scalar(address),
            Value::Variant { variant, fields } => Value::Variant {
                variant: *variant,
                fields: fields.iter().map(|field| substitute(field, address)).collect(),
            },
            Value::WidePointer { data, metadata } => Value::WidePointer {
                data: Box::new(substitute(data, address)),
                metadata: metadata.clone(),
            },
            other => other.clone(),
        }
    }
    let arguments: Vec<_> = arguments
        .iter()
        .map(|argument| substitute(argument, address))
        .collect();
    let mut callee = CallEffects {
        write: write.map(|value| (address, value)),
        calls: 0,
    };
    let outcome = Interpreter::new(graph).run_values_in_memory(&arguments, &mut callee, &mut memory);
    (outcome, callee.calls)
}

fn optimize(graph: &mut FunctionGraph) -> OptimizeStats {
    optimize_specialized_graph(graph).unwrap()
}

/// `fn reload(x: P) -> u64 { let a = *x; f(); let b = *x; a + b }` (P5.1).
fn reload(pointer: impl FnOnce(&mut Types) -> TyId) -> Body {
    let mut t = types();
    let pointer = pointer(&mut t);
    let (word, unit) = (t.word, t.unit);
    let mut b = BodyBuilder::new(
        "reload",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        t.types,
    );
    let ret = local(&mut b, word);
    let x = local(&mut b, pointer);
    b.set_arg_count(1);
    let first = local(&mut b, word);
    let second = local(&mut b, word);
    let nothing = local(&mut b, unit);
    let place = deref(&mut b, x);
    assign(&mut b, Place::local(first), Rvalue::Use(copy(place)));
    call(&mut b, unit, "f", vec![], nothing, 1);
    let place = deref(&mut b, x);
    assign(&mut b, Place::local(second), Rvalue::Use(copy(place)));
    add(&mut b, ret, first, second);
    b.finish_block(Terminator::Return, false);
    b.finish()
}

#[test]
fn frozen_parameter_load_is_reused_across_an_unknown_call() {
    let body = reload(|t| t.reference(Mutability::Not, t.word));
    let mut graph = graph(body, vec![FROZEN_WORD]);
    let before = run(&graph, &[Value::Scalar(ADDRESS)], 20, None);
    assert_eq!(before.0, Ok(SemanticOutcome::Return(Value::Scalar(40))));
    let stats = optimize(&mut graph);
    assert_eq!(loads(&graph), 1);
    assert_eq!(stats.frozen_loads_reused, 1);
    assert_eq!(stats.calls_crossed_by_forwarding, 1);
    assert_eq!(run(&graph, &[Value::Scalar(ADDRESS)], 20, None), before);
}

#[test]
fn reload_without_a_frozen_permission_observes_the_call() {
    // N5.1–N5.3: `&Cell`/`&RefCell` (not Freeze), `&mut T` (Unique has no
    // consumer yet), raw pointers and the `off` switch all carry no Frozen.
    let cases: Vec<(Box<dyn FnOnce(&mut Types) -> TyId>, ArgPermission, bool)> = vec![
        (
            Box::new(|t: &mut Types| t.reference(Mutability::Not, t.word)),
            ArgPermission::None,
            true,
        ),
        (
            Box::new(|t: &mut Types| t.reference(Mutability::Mut, t.word)),
            ArgPermission::Unique {
                dereferenceable: Some(8),
            },
            false,
        ),
        (
            Box::new(|t: &mut Types| t.raw(Mutability::Not, t.word)),
            ArgPermission::None,
            true,
        ),
    ];
    for (pointer, permission, writes) in cases {
        let mut graph = graph(reload(pointer), vec![permission]);
        // A call may write the place when no permission forbids it.
        let write = writes.then_some(91);
        let before = run(&graph, &[Value::Scalar(ADDRESS)], 20, write);
        if writes {
            assert_eq!(before.0, Ok(SemanticOutcome::Return(Value::Scalar(111))));
        }
        let stats = optimize(&mut graph);
        assert_eq!(loads(&graph), 2, "{permission:?}");
        assert_eq!(stats.frozen_loads_reused, 0);
        assert_eq!(run(&graph, &[Value::Scalar(ADDRESS)], 20, write), before);
    }
}

#[test]
fn verifier_admits_permissions_only_on_matching_specialized_references() {
    let shared = reload(|t| t.reference(Mutability::Not, t.word));
    let mutable = reload(|t| t.reference(Mutability::Mut, t.word));
    let raw = reload(|t| t.raw(Mutability::Not, t.word));
    let unique = ArgPermission::Unique {
        dereferenceable: Some(8),
    };
    for (body, permissions) in [
        (&raw, vec![FROZEN_WORD]),
        (&shared, vec![unique]),
        (&mutable, vec![FROZEN_WORD]),
        (&shared, vec![FROZEN_WORD, ArgPermission::None]),
    ] {
        let mut graph = build_graph(body).unwrap();
        graph.arg_permissions = permissions.clone();
        assert!(verify_graph(&mut graph).is_err(), "{permissions:?}");
    }
    let mut graph = build_graph(&shared).unwrap();
    graph.arg_permissions = vec![FROZEN_WORD];
    graph.level = GraphLevel::Rust;
    assert!(verify_graph(&mut graph).is_err(), "R carries no permissions");
    let mut graph = build_graph(&mutable).unwrap();
    graph.arg_permissions = vec![unique];
    verify_graph(&mut graph).unwrap();
    assert!(graph.dump().contains("permissions p0:Unique(8)"));
}

#[test]
fn layout_lowering_erases_permissions() {
    let mut graph = graph(reload(|t| t.reference(Mutability::Not, t.word)), vec![FROZEN_WORD]);
    graph
        .callee_permissions
        .insert("f".into(), Vec::new());
    optimize(&mut graph);
    verify_graph(&mut graph).unwrap();
    kfrc_son::lower_layout(&mut graph).unwrap();
    assert!(graph.arg_permissions.is_empty());
    assert!(graph.callee_permissions.is_empty());
    verify_graph(&mut graph).unwrap();
}

/// `fn nested(x: &*mut u64) -> u64 { let a = **x; f(); let b = **x; a + b }`:
/// the outer pointer is Frozen, the pointer read from it is not.
#[test]
fn pointer_loaded_from_frozen_memory_has_no_permission() {
    let mut t = types();
    let inner = t.raw(Mutability::Mut, t.word);
    let outer = t.reference(Mutability::Not, inner);
    let (word, unit) = (t.word, t.unit);
    let mut b = BodyBuilder::new(
        "nested",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        t.types,
    );
    let ret = local(&mut b, word);
    let x = local(&mut b, outer);
    b.set_arg_count(1);
    let first = local(&mut b, word);
    let second = local(&mut b, word);
    let nothing = local(&mut b, unit);
    let twice = b.add_projection(vec![ProjectionElem::Deref, ProjectionElem::Deref]);
    assign(
        &mut b,
        Place::local(first),
        Rvalue::Use(copy(Place {
            local: x,
            projection: twice,
        })),
    );
    call(&mut b, unit, "f", vec![], nothing, 1);
    assign(
        &mut b,
        Place::local(second),
        Rvalue::Use(copy(Place {
            local: x,
            projection: twice,
        })),
    );
    add(&mut b, ret, first, second);
    b.finish_block(Terminator::Return, false);
    let mut graph = graph(b.finish(), vec![FROZEN_WORD]);
    // The outer word holds the inner pointer: store it in a second allocation.
    let outcome = |graph: &FunctionGraph| {
        let mut memory = SemanticMemory::new();
        let target = memory.allocate(8, 8, true).unwrap();
        let holder = memory.allocate(8, 8, true).unwrap();
        let word = |value: u64| -> Vec<_> { value.to_le_bytes().into_iter().map(Some).collect() };
        memory.write_bytes(target, &word(20)).unwrap();
        memory.write_bytes(holder, &word(target)).unwrap();
        let mut callee = CallEffects {
            write: Some((target, 91)),
            calls: 0,
        };
        Interpreter::new(graph).run_values_in_memory(&[Value::Scalar(holder)], &mut callee, &mut memory)
    };
    let before = outcome(&graph);
    assert_eq!(before, Ok(SemanticOutcome::Return(Value::Scalar(111))));
    let stats = optimize(&mut graph);
    // The second outer read reuses the first; both inner reads remain.
    assert_eq!(loads(&graph), 3);
    assert_eq!(stats.frozen_loads_reused, 1);
    assert_eq!(outcome(&graph), before);
}

/// `fn local(f) -> u64 { let x = 3; observe(&x); x }` (P5.3, Q3): `observe`
/// receives `&u64` with the given call-site permission.
fn observed_local(permission: Vec<ArgPermission>, pass_field: bool) -> FunctionGraph {
    let mut t = types();
    let pair = t.types.intern(
        TypeShape::Tuple(vec![t.word, t.word]),
        Some(SizeAlign { size: 16, align: 8 }),
    );
    t.types.record_aggregate(
        pair,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: t.word,
                    offset: 0,
                },
                FieldLayout {
                    ty: t.word,
                    offset: 8,
                },
            ],
            variants: vec![],
        },
    );
    let shared = t.reference(Mutability::Not, t.word);
    let (word, unit) = (t.word, t.unit);
    let mut b = BodyBuilder::new(
        "observed",
        FunctionAbi {
            args: vec![],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        t.types,
    );
    let ret = local(&mut b, word);
    let x = local(&mut b, pair);
    let borrowed = local(&mut b, shared);
    let nothing = local(&mut b, unit);
    let three = scalar(&mut b, word, 3);
    let four = scalar(&mut b, word, 4);
    let field = |b: &mut BodyBuilder, index: u32| Place {
        local: x,
        projection: b.add_projection(vec![ProjectionElem::Field(FieldIndex(index), word)]),
    };
    let second = field(&mut b, 1);
    assign(&mut b, second, Rvalue::Use(four));
    let first = field(&mut b, 0);
    assign(&mut b, first, Rvalue::Use(three));
    let passed = field(&mut b, u32::from(pass_field));
    assign(
        &mut b,
        Place::local(borrowed),
        Rvalue::Ref(BorrowKind::Shared, passed),
    );
    call(
        &mut b,
        unit,
        "observe",
        vec![copy(Place::local(borrowed))],
        nothing,
        1,
    );
    let read = field(&mut b, 0);
    assign(&mut b, Place::local(ret), Rvalue::Use(copy(read)));
    b.finish_block(Terminator::Return, false);
    let mut graph = graph(b.finish(), Vec::new());
    graph.callee_permissions.insert("observe".into(), permission);
    verify_graph(&mut graph).unwrap();
    graph
}

#[test]
fn call_site_frozen_argument_preserves_its_place() {
    let mut graph = observed_local(vec![FROZEN_WORD], false);
    let before = run(&graph, &[], 0, None);
    assert_eq!(before.0, Ok(SemanticOutcome::Return(Value::Scalar(3))));
    let stats = optimize(&mut graph);
    assert_eq!(loads(&graph), 0, "{}", graph.dump());
    assert_eq!(stats.call_permissions_used, 1);
    assert_eq!(run(&graph, &[], 0, None), before);
}

#[test]
fn call_site_permission_covers_only_its_own_place() {
    // N5.7 shapes: no permission, an unsized range, and a different field.
    for (permission, pass_field) in [
        (vec![ArgPermission::None], false),
        (
            vec![ArgPermission::Frozen {
                dereferenceable: None,
            }],
            false,
        ),
        (vec![FROZEN_WORD], true),
    ] {
        let mut graph = observed_local(permission.clone(), pass_field);
        let before = run(&graph, &[], 0, None);
        let stats = optimize(&mut graph);
        assert_eq!(loads(&graph), 1, "{permission:?} {pass_field}");
        assert_eq!(stats.call_permissions_used, 0);
        assert_eq!(run(&graph, &[], 0, None), before);
    }
}

/// `fn sum(x: &u64, n: u64) -> u64 { let a = *x; let mut i = 0; let mut s = a;
/// while i != n { f(); s += *x; i += 1 } s }`: the loop body reuses `a`.
fn frozen_loop(permission: ArgPermission) -> FunctionGraph {
    let mut t = types();
    let shared = t.reference(Mutability::Not, t.word);
    let (word, unit, boolean) = (t.word, t.unit, t.boolean);
    let mut b = BodyBuilder::new(
        "frozen_loop",
        FunctionAbi {
            args: vec![PassMode::Direct, PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        t.types,
    );
    let ret = local(&mut b, word);
    let x = local(&mut b, shared);
    let n = local(&mut b, word);
    b.set_arg_count(2);
    let index = local(&mut b, word);
    let value = local(&mut b, word);
    let done = local(&mut b, boolean);
    let nothing = local(&mut b, unit);
    let zero = scalar(&mut b, word, 0);
    let one = scalar(&mut b, word, 1);
    // bb0: entry
    let place = deref(&mut b, x);
    assign(&mut b, Place::local(ret), Rvalue::Use(copy(place)));
    assign(&mut b, Place::local(index), Rvalue::Use(zero));
    b.finish_block(Terminator::Goto(BlockId(1)), false);
    // bb1: header
    assign(
        &mut b,
        Place::local(done),
        Rvalue::BinaryOp(BinaryOp::Eq, copy(Place::local(index)), copy(Place::local(n))),
    );
    b.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(Place::local(done)),
            targets: SwitchTargets {
                cases: vec![(0, BlockId(2))],
                otherwise: BlockId(4),
            },
        },
        false,
    );
    // bb2: body call
    call(&mut b, unit, "f", vec![], nothing, 3);
    // bb3: body read
    let place = deref(&mut b, x);
    assign(&mut b, Place::local(value), Rvalue::Use(copy(place)));
    add(&mut b, ret, ret, value);
    assign(
        &mut b,
        Place::local(index),
        Rvalue::BinaryOp(BinaryOp::Add, copy(Place::local(index)), one),
    );
    b.finish_block(Terminator::Goto(BlockId(1)), false);
    // bb4: exit
    b.finish_block(Terminator::Return, false);
    graph(b.finish(), vec![permission, ArgPermission::None])
}

#[test]
fn frozen_load_before_a_loop_serves_every_iteration() {
    let mut graph = frozen_loop(FROZEN_WORD);
    let outcomes = |graph: &FunctionGraph| {
        (0..4)
            .map(|n| run(graph, &[Value::Scalar(ADDRESS), Value::Scalar(n)], 5, None))
            .collect::<Vec<_>>()
    };
    let before = outcomes(&graph);
    assert_eq!(before[3].0, Ok(SemanticOutcome::Return(Value::Scalar(20))));
    let stats = optimize(&mut graph);
    assert_eq!(loads(&graph), 1, "{}", graph.dump());
    assert_eq!(stats.frozen_loads_reused, 1);
    assert!(stats.effect_phis_crossed >= 1);
    assert_eq!(outcomes(&graph), before);
}

#[test]
fn loop_reads_stay_when_the_parameter_is_not_frozen() {
    let mut graph = frozen_loop(ArgPermission::None);
    let outcomes = |graph: &FunctionGraph| {
        (0..4)
            .map(|n| run(graph, &[Value::Scalar(ADDRESS), Value::Scalar(n)], 5, Some(7)))
            .collect::<Vec<_>>()
    };
    let before = outcomes(&graph);
    assert_eq!(before[3].0, Ok(SemanticOutcome::Return(Value::Scalar(26))));
    optimize(&mut graph);
    assert_eq!(loads(&graph), 2);
    assert_eq!(outcomes(&graph), before);
}

/// `match o { Some(p) => if *p >= 65 && *p <= 90 { 1 } else if *p == 45 { 2 }
/// else { 3 }, None => 0 }` with every `*p` re-projected from the enum, as
/// rustc's MIR does for match arms. Reads need no permission to be reused.
#[test]
fn match_arm_rereads_of_one_variant_field_share_the_first_read() {
    let mut t = types();
    let shared = t.reference(Mutability::Not, t.word);
    let option = t.types.intern(
        TypeShape::Adt {
            name: "Option<&u64>".into(),
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    t.types.record_semantic_aggregate(
        option,
        SemanticAggregate {
            fields: vec![],
            variants: vec![vec![], vec![shared]],
        },
    );
    t.types.record_semantic_enum(option, vec![0, 1]);
    let discriminant = t.types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let (word, boolean) = (t.word, t.boolean);
    let mut b = BodyBuilder::new(
        "match_arms",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        t.types,
    );
    let ret = local(&mut b, word);
    let o = local(&mut b, option);
    b.set_arg_count(1);
    let tag = local(&mut b, discriminant);
    let byte = local(&mut b, word);
    let test = local(&mut b, boolean);
    let pointee = b.add_projection(vec![
        ProjectionElem::Downcast(VariantIndex(1)),
        ProjectionElem::Field(FieldIndex(0), shared),
        ProjectionElem::Deref,
    ]);
    let read = |b: &mut BodyBuilder, compare: BinaryOp, bound: u128| {
        assign(
            b,
            Place::local(byte),
            Rvalue::Use(copy(Place {
                local: o,
                projection: pointee,
            })),
        );
        let bound = scalar(b, word, bound);
        assign(
            b,
            Place::local(test),
            Rvalue::BinaryOp(compare, copy(Place::local(byte)), bound),
        );
    };
    let branch = |b: &mut BodyBuilder, yes: u32, no: u32| {
        b.finish_block(
            Terminator::SwitchInt {
                discriminant: copy(Place::local(test)),
                targets: SwitchTargets {
                    cases: vec![(0, BlockId(no))],
                    otherwise: BlockId(yes),
                },
            },
            false,
        );
    };
    let result = |b: &mut BodyBuilder, value: u128| {
        let value = scalar(b, word, value);
        assign(b, Place::local(ret), Rvalue::Use(value));
        b.finish_block(Terminator::Return, false);
    };
    // bb0: switch on the variant
    assign(
        &mut b,
        Place::local(tag),
        Rvalue::Discriminant(Place::local(o)),
    );
    b.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(Place::local(tag)),
            targets: SwitchTargets {
                cases: vec![(1, BlockId(1))],
                otherwise: BlockId(4),
            },
        },
        false,
    );
    // bb1: *p >= 65
    read(&mut b, BinaryOp::Ge, 65);
    branch(&mut b, 2, 3);
    // bb2: *p <= 90
    read(&mut b, BinaryOp::Le, 90);
    branch(&mut b, 5, 3);
    // bb3: *p == 45, reached from both earlier reads
    read(&mut b, BinaryOp::Eq, 45);
    branch(&mut b, 6, 7);
    // bb4..bb7: results
    result(&mut b, 0);
    result(&mut b, 1);
    result(&mut b, 2);
    result(&mut b, 3);
    let mut graph = graph(b.finish(), Vec::new());
    let outcomes = |graph: &FunctionGraph| {
        [70u64, 50, 45, 100, 65, 90, 91]
            .map(|value| {
                let some = Value::Variant {
                    variant: 1,
                    fields: vec![Value::Scalar(ADDRESS)],
                };
                run(graph, &[some], value, None).0
            })
            .to_vec()
    };
    let before = outcomes(&graph);
    assert_eq!(before[0], Ok(SemanticOutcome::Return(Value::Scalar(1))));
    assert_eq!(before[2], Ok(SemanticOutcome::Return(Value::Scalar(2))));
    assert_eq!(before[3], Ok(SemanticOutcome::Return(Value::Scalar(3))));
    assert_eq!(loads(&graph), 3);
    let stats = optimize(&mut graph);
    assert_eq!(loads(&graph), 1, "{}", graph.dump());
    assert_eq!(stats.loads_forwarded, 2);
    assert_eq!(stats.frozen_loads_reused, 0);
    assert!(stats.effect_phis_crossed >= 1);
    assert_eq!(outcomes(&graph), before);
}

/// `fn s2(s: &[u64]) -> u64 { let a = s[0]; f(); a + s[0] }` (P5.2): the
/// data pointer of a wide Frozen reference keeps the permission.
#[test]
fn frozen_slice_element_is_reused_across_a_call() {
    for permission in [
        ArgPermission::Frozen {
            dereferenceable: None,
        },
        ArgPermission::None,
    ] {
        let mut t = types();
        let word = t.word;
        let slice = t.types.intern(TypeShape::Slice { element: word }, None);
        let data = t.raw(Mutability::Not, word);
        let wide = t.types.intern(
            TypeShape::Reference {
                mutability: Mutability::Not,
                pointee: slice,
            },
            None,
        );
        t.types.record_semantic_aggregate(
            wide,
            SemanticAggregate {
                fields: vec![data, word],
                variants: vec![],
            },
        );
        let unit = t.unit;
        let mut b = BodyBuilder::new(
            "frozen_slice",
            FunctionAbi {
                args: vec![PassMode::Pair],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            t.types,
        );
        let ret = local(&mut b, word);
        let s = local(&mut b, wide);
        b.set_arg_count(1);
        let first = local(&mut b, word);
        let second = local(&mut b, word);
        let nothing = local(&mut b, unit);
        let element = b.add_projection(vec![
            ProjectionElem::Deref,
            ProjectionElem::ConstantIndex {
                offset: 0,
                min_length: 1,
                from_end: false,
            },
        ]);
        let place = Place {
            local: s,
            projection: element,
        };
        assign(&mut b, Place::local(first), Rvalue::Use(copy(place)));
        call(&mut b, unit, "f", vec![], nothing, 1);
        assign(&mut b, Place::local(second), Rvalue::Use(copy(place)));
        add(&mut b, ret, first, second);
        b.finish_block(Terminator::Return, false);
        let mut graph = graph(b.finish(), vec![permission]);
        let argument = Value::WidePointer {
            data: Box::new(Value::Scalar(ADDRESS)),
            metadata: Box::new(Value::Scalar(2)),
        };
        let write = (permission == ArgPermission::None).then_some(9);
        let before = run(&graph, std::slice::from_ref(&argument), 21, write);
        let sum = if permission.is_frozen() { 42 } else { 30 };
        assert_eq!(before.0, Ok(SemanticOutcome::Return(Value::Scalar(sum))));
        let stats = optimize(&mut graph);
        let expected = if permission.is_frozen() { 1 } else { 2 };
        assert_eq!(loads(&graph), expected, "{}", graph.dump());
        assert_eq!(stats.frozen_loads_reused, 2 - expected);
        assert_eq!(run(&graph, std::slice::from_ref(&argument), 21, write), before);
    }
}

/// Inlining v1: call-site permissions travel with the copied calls; the
/// callee's own parameter permissions end at the seam.
#[test]
fn inlining_copies_call_site_permissions_but_not_callee_parameters() {
    let mut callee = graph(
        reload(|t| t.reference(Mutability::Not, t.word)),
        vec![FROZEN_WORD],
    );
    callee
        .callee_permissions
        .insert("f".into(), Vec::new());
    optimize(&mut callee);
    assert_eq!(loads(&callee), 1);
    let mut t = types();
    let shared = t.reference(Mutability::Not, t.word);
    let word = t.word;
    let mut b = BodyBuilder::new(
        "caller",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        t.types,
    );
    let ret = local(&mut b, word);
    let y = local(&mut b, shared);
    b.set_arg_count(1);
    let target = Operand::Constant(b.add_constant(Constant {
        ty: word,
        kind: ConstantKind::Function(Callee::Function {
            symbol: "reload".into(),
            abi: callee.abi.clone(),
        }),
    }));
    b.finish_block(
        Terminator::Call {
            callee: target,
            args: vec![copy(Place::local(y))],
            destination: Place::local(ret),
            target: Some(BlockId(1)),
            unwind: UnwindAction::Unreachable,
        },
        false,
    );
    b.finish_block(Terminator::Return, false);
    let mut caller = graph(b.finish(), vec![ArgPermission::None]);
    let provider = std::collections::BTreeMap::from([("reload".to_owned(), callee)]);
    let stats =
        kfrc_son::inline_calls(&mut caller, &provider, kfrc_son::InlineBudget::default()).unwrap();
    assert_eq!(stats.calls_inlined, 1);
    verify_graph(&mut caller).unwrap();
    assert_eq!(caller.arg_permissions, vec![ArgPermission::None]);
    assert_eq!(caller.callee_permissions.get("f"), Some(&Vec::new()));
    assert_eq!(loads(&caller), 1);
    let before = run(&provider["reload"], &[Value::Scalar(ADDRESS)], 20, None);
    assert_eq!(run(&caller, &[Value::Scalar(ADDRESS)], 20, None), before);
}
