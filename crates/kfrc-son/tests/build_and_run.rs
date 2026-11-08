//! Body → R graph → verifier → interpreter on hand-built scalar bodies.

use kfrc_body::*;
use kfrc_son::{
    BuildError, FunctionGraph, InterpretError, Interpreter, NodeId, Opcode, Outcome, ValueType,
    build_graph, verify::check, verify_graph,
};

struct Types {
    recorded: RecordedTypes,
    i64: TyId,
    u8: TyId,
    i8: TyId,
    bool: TyId,
    unit: TyId,
}

fn types() -> Types {
    let mut recorded = RecordedTypes::new();
    let i64 = recorded.intern(TypeShape::Int { width: IntegerWidth::W64, signed: true }, Some(SizeAlign { size: 8, align: 8 }));
    let u8 = recorded.intern(TypeShape::Int { width: IntegerWidth::W8, signed: false }, Some(SizeAlign { size: 1, align: 1 }));
    let i8 = recorded.intern(TypeShape::Int { width: IntegerWidth::W8, signed: true }, Some(SizeAlign { size: 1, align: 1 }));
    let bool = recorded.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let unit = recorded.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
    Types { recorded, i64, u8, i8, bool, unit }
}

fn local(builder: &mut BodyBuilder, ty: TyId) -> LocalId {
    builder.add_local(LocalDecl { ty, mutability: Mutability::Mut, name: None })
}

fn copy(local: LocalId) -> Operand {
    Operand::Copy(Place::local(local))
}

fn constant(builder: &mut BodyBuilder, ty: TyId, bits: u128) -> Operand {
    Operand::Constant(builder.add_constant(Constant { ty, kind: ConstantKind::Scalar(bits) }))
}

fn assign(builder: &mut BodyBuilder, target: LocalId, rvalue: Rvalue) {
    builder.push_statement(StatementKind::Assign(Place::local(target), rvalue));
}

fn abi(args: usize) -> FunctionAbi {
    FunctionAbi { args: vec![PassMode::Direct; args], ret: PassMode::Direct, c_variadic: false }
}

fn run(graph: &FunctionGraph, args: &[u64]) -> Outcome {
    let mut no_calls = |symbol: &str, _: &[u64]| Err(InterpretError::UnknownCallee(symbol.to_owned()));
    Interpreter::new(graph).run(args, &mut no_calls).expect("interpretation succeeds")
}

fn build_verified(body: &Body) -> FunctionGraph {
    let mut graph = build_graph(body).unwrap_or_else(|error| panic!("{error}\n{body}"));
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    graph
}

/// The T02 running example: `let mut x = a; if f { x = x + 1; } x`.
#[test]
fn add_if_merges_branch_assignment() {
    let t = types();
    let mut builder = BodyBuilder::new("add_if", abi(2), t.recorded.clone());
    let ret = local(&mut builder, t.i64);
    let f = local(&mut builder, t.bool);
    let a = local(&mut builder, t.i64);
    builder.set_arg_count(2);
    let x = local(&mut builder, t.i64);
    assign(&mut builder, x, Rvalue::Use(copy(a)));
    builder.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(f),
            targets: SwitchTargets { cases: vec![(0, BlockId(2))], otherwise: BlockId(1) },
        },
        false,
    );
    let one = constant(&mut builder, t.i64, 1);
    assign(&mut builder, x, Rvalue::BinaryOp(BinaryOp::Add, copy(x), one));
    builder.finish_block(Terminator::Goto(BlockId(2)), false);
    assign(&mut builder, ret, Rvalue::Use(copy(x)));
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    assert_eq!(run(&graph, &[1, 41]), Outcome::Return(42));
    assert_eq!(run(&graph, &[0, 41]), Outcome::Return(41));
    assert!(graph.ids().any(|node| graph.op(node) == Opcode::Phi), "{}", graph.dump());
}

/// `let (mut i, mut s) = (0, 0); while i < n { s += i; i += 1 } s`
#[test]
fn while_loop_carries_two_variables() {
    let t = types();
    let mut builder = BodyBuilder::new("sum", abi(1), t.recorded.clone());
    let ret = local(&mut builder, t.i64);
    let n = local(&mut builder, t.i64);
    builder.set_arg_count(1);
    let i = local(&mut builder, t.i64);
    let s = local(&mut builder, t.i64);
    let c = local(&mut builder, t.bool);
    let zero = constant(&mut builder, t.i64, 0);
    assign(&mut builder, i, Rvalue::Use(zero));
    assign(&mut builder, s, Rvalue::Use(zero));
    builder.finish_block(Terminator::Goto(BlockId(1)), false);
    assign(&mut builder, c, Rvalue::BinaryOp(BinaryOp::Lt, copy(i), copy(n)));
    builder.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(c),
            targets: SwitchTargets { cases: vec![(0, BlockId(3))], otherwise: BlockId(2) },
        },
        false,
    );
    let one = constant(&mut builder, t.i64, 1);
    assign(&mut builder, s, Rvalue::BinaryOp(BinaryOp::Add, copy(s), copy(i)));
    assign(&mut builder, i, Rvalue::BinaryOp(BinaryOp::Add, copy(i), one));
    builder.finish_block(Terminator::Goto(BlockId(1)), false);
    assign(&mut builder, ret, Rvalue::Use(copy(s)));
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    assert_eq!(run(&graph, &[10]), Outcome::Return(45));
    assert_eq!(run(&graph, &[0]), Outcome::Return(0));
}

/// `match x { 0 => 10, 7 => 20, _ => 30 }` over `u8`.
#[test]
fn multiway_switch() {
    let t = types();
    let mut builder = BodyBuilder::new("pick", abi(1), t.recorded.clone());
    let ret = local(&mut builder, t.i64);
    let x = local(&mut builder, t.u8);
    builder.set_arg_count(1);
    builder.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(x),
            targets: SwitchTargets { cases: vec![(0, BlockId(1)), (7, BlockId(2))], otherwise: BlockId(3) },
        },
        false,
    );
    for value in [10, 20, 30] {
        let value = constant(&mut builder, t.i64, value);
        assign(&mut builder, ret, Rvalue::Use(value));
        builder.finish_block(Terminator::Goto(BlockId(4)), false);
    }
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    assert_eq!(run(&graph, &[0]), Outcome::Return(10));
    assert_eq!(run(&graph, &[7]), Outcome::Return(20));
    assert_eq!(run(&graph, &[0x107]), Outcome::Return(20), "upper argument bits are ignored for u8");
    assert_eq!(run(&graph, &[3]), Outcome::Return(30));
}

#[test]
fn narrow_integers_wrap_and_cast() {
    let t = types();
    let mut builder = BodyBuilder::new("narrow", abi(2), t.recorded.clone());
    let ret = local(&mut builder, t.i64);
    let a = local(&mut builder, t.u8);
    let b = local(&mut builder, t.i8);
    builder.set_arg_count(2);
    let sum = local(&mut builder, t.u8);
    let difference = local(&mut builder, t.i8);
    let wide_sum = local(&mut builder, t.i64);
    let wide_difference = local(&mut builder, t.i64);
    let one = constant(&mut builder, t.i8, 1);
    assign(&mut builder, sum, Rvalue::BinaryOp(BinaryOp::Add, copy(a), copy(a)));
    assign(&mut builder, difference, Rvalue::BinaryOp(BinaryOp::Sub, copy(b), one));
    assign(&mut builder, wide_sum, Rvalue::Cast(CastKind::IntToInt, copy(sum), t.i64));
    assign(&mut builder, wide_difference, Rvalue::Cast(CastKind::IntToInt, copy(difference), t.i64));
    let thousand = constant(&mut builder, t.i64, 1000);
    let scaled = local(&mut builder, t.i64);
    assign(&mut builder, scaled, Rvalue::BinaryOp(BinaryOp::Mul, copy(wide_sum), thousand));
    assign(&mut builder, ret, Rvalue::BinaryOp(BinaryOp::Add, copy(scaled), copy(wide_difference)));
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    // (200 + 200) as u8 = 144; (-128i8 - 1) wraps to 127.
    assert_eq!(run(&graph, &[200, 0x80]), Outcome::Return(144 * 1000 + 127));
}

#[test]
fn calls_thread_the_effect_token() {
    let t = types();
    let mut builder = BodyBuilder::new("twice", abi(1), t.recorded.clone());
    let ret = local(&mut builder, t.i64);
    let x = local(&mut builder, t.i64);
    builder.set_arg_count(1);
    let first = local(&mut builder, t.i64);
    let callee = Callee::Function { symbol: "inc".to_owned(), abi: abi(1) };
    let callee = Operand::Constant(builder.add_constant(Constant { ty: t.unit, kind: ConstantKind::Function(callee) }));
    builder.finish_block(
        Terminator::Call { callee, args: vec![copy(x)], destination: Place::local(first), target: Some(BlockId(1)), unwind: UnwindAction::Unreachable },
        false,
    );
    builder.finish_block(
        Terminator::Call { callee, args: vec![copy(first)], destination: Place::local(ret), target: Some(BlockId(2)), unwind: UnwindAction::Unreachable },
        false,
    );
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    let mut log = Vec::new();
    let mut handler = |symbol: &str, args: &[u64]| {
        log.push((symbol.to_owned(), args.to_vec()));
        Ok(args[0] + 1)
    };
    let outcome = Interpreter::new(&graph).run(&[5], &mut handler).unwrap();
    assert_eq!(outcome, Outcome::Return(7));
    assert_eq!(log, vec![("inc".to_owned(), vec![5]), ("inc".to_owned(), vec![6])]);
}

#[test]
fn assert_terminator_is_reported_unsupported() {
    let t = types();
    let mut builder = BodyBuilder::new("checked", abi(1), t.recorded.clone());
    let _ret = local(&mut builder, t.i64);
    let x = local(&mut builder, t.bool);
    builder.set_arg_count(1);
    builder.finish_block(
        Terminator::Assert { condition: copy(x), expected: true, message: AssertKind::DivisionByZero, target: BlockId(1), unwind: UnwindAction::Unreachable },
        false,
    );
    builder.finish_block(Terminator::Return, false);
    match build_graph(&builder.finish()) {
        Err(BuildError::Unsupported { what, .. }) => assert!(what.contains("Assert"), "{what}"),
        other => panic!("expected unsupported, got {other:?}"),
    }
}

/// The T01 miscompile shape built by hand: a value defined in one branch is
/// used after the merge without a phi. The verifier must reject it.
#[test]
fn verifier_rejects_branch_value_used_after_merge() {
    let t = types();
    let mut graph = FunctionGraph::new("bad", abi(2), t.recorded.clone());
    let start = graph.add_node(Opcode::Start, NodeId::NONE, &[], 0, ValueType::Tuple(vec![ValueType::Control, ValueType::Effect, ValueType::Rust(t.bool), ValueType::Rust(t.i64)]));
    graph.start = start;
    let control = graph.add_node(Opcode::Projection, NodeId::NONE, &[start], 0, ValueType::Control);
    let effect = graph.add_node(Opcode::Projection, NodeId::NONE, &[start], 1, ValueType::Effect);
    let flag = graph.add_node(Opcode::Projection, NodeId::NONE, &[start], 2, ValueType::Rust(t.bool));
    let a = graph.add_node(Opcode::Projection, NodeId::NONE, &[start], 3, ValueType::Rust(t.i64));
    let branch = graph.add_node(Opcode::If, control, &[flag], 0, ValueType::Tuple(vec![ValueType::Control, ValueType::Control]));
    let taken = graph.add_node(Opcode::Projection, NodeId::NONE, &[branch], 0, ValueType::Control);
    let skipped = graph.add_node(Opcode::Projection, NodeId::NONE, &[branch], 1, ValueType::Control);
    let zero = graph.add_constant(0, t.i64);
    // A trapping division fixed in the taken branch.
    let quotient = graph.add_node(Opcode::DivSigned, taken, &[a, a], 0, ValueType::Rust(t.i64));
    let jump_taken = graph.add_node(Opcode::Jump, taken, &[], 0, ValueType::Control);
    let jump_skipped = graph.add_node(Opcode::Jump, skipped, &[], 0, ValueType::Control);
    let merge = graph.add_growable_node(Opcode::Region, NodeId::NONE, 2, 0, ValueType::Control);
    graph.set_input(merge, 0, jump_taken);
    graph.set_input(merge, 1, jump_skipped);
    let sum = graph.add_node(Opcode::Add, NodeId::NONE, &[quotient, zero], 0, ValueType::Rust(t.i64));
    graph.add_node(Opcode::Return, merge, &[effect, sum], 0, ValueType::Control);
    match check(&graph) {
        Err(kfrc_son::VerifyError::DominanceViolation { .. }) => {}
        other => panic!("expected a dominance violation, got {other:?}\n{}", graph.dump()),
    }
}
