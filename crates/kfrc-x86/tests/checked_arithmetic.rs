//! Run encoded checked products/sums, including values crossing a success edge.
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TypeShape};
use kfrc_son::{
    ArithmeticFlags, ComparePredicate, FunctionGraph, GraphLevel, NodeId, Opcode, ValueType,
    lower_layout, verify_graph,
};
use kfrc_x86::{
    lower_function,
    machine::x86::{MachineFunction, MachineInstr},
};

fn checked(
    symbol: &str,
    op: Opcode,
    guarded: bool,
    signed: bool,
    width: IntegerWidth,
) -> MachineFunction {
    let (mut g, ty, boolean, control, effect, a, b) = setup(symbol, signed, width);
    // Create the value before its independent overflow node, as the frontend does.
    let value = g.add_node(op, NodeId::NONE, &[a, b], 0, ValueType::Rust(ty));
    let overflow = if op == Opcode::Mul {
        g.add_node(
            Opcode::MulOverflow,
            NodeId::NONE,
            &[a, b],
            0,
            ValueType::Rust(boolean),
        )
    } else {
        g.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[value, a],
            ComparePredicate::UnsignedLess as u32,
            ValueType::Rust(boolean),
        )
    };
    if guarded {
        let branch = g.add_node(
            Opcode::If,
            control,
            &[overflow],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
        );
        let yes = g.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            0,
            ValueType::Control,
        );
        let no = g.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            1,
            ValueType::Control,
        );
        let sentinel = g.add_constant(37, ty);
        g.add_node(
            Opcode::Return,
            yes,
            &[effect, sentinel],
            0,
            ValueType::Control,
        );
        // The guarded value is a separate node with a proven no-overflow contract.
        let result = g.add_node(op, no, &[b, a], ArithmeticFlags::NUW, ValueType::Rust(ty));
        g.add_node(Opcode::Return, no, &[effect, result], 0, ValueType::Control);
    } else {
        // Both results have multiple consumers; materialize overflow before a
        // flag-clobbering XOR and preserve the full (possibly wrapped) product.
        let flag = g.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[overflow],
            0,
            ValueType::Rust(ty),
        );
        let result = g.add_node(
            Opcode::BitXor,
            NodeId::NONE,
            &[value, flag],
            0,
            ValueType::Rust(ty),
        );
        g.add_node(
            Opcode::Return,
            control,
            &[effect, result],
            0,
            ValueType::Control,
        );
    }
    verify_graph(&mut g).unwrap();
    lower_layout(&mut g).unwrap();
    verify_graph(&mut g).unwrap();
    let function = lower_function(&g).unwrap();
    assert!(function.loop_headers.is_empty());
    function
}

fn setup(
    symbol: &str,
    signed: bool,
    width: IntegerWidth,
) -> (
    FunctionGraph,
    kfrc_body::TyId,
    kfrc_body::TyId,
    NodeId,
    NodeId,
    NodeId,
    NodeId,
) {
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
    let mut g = FunctionGraph::new(
        symbol,
        FunctionAbi {
            args: vec![PassMode::Direct; 2],
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
            ValueType::Rust(ty),
            ValueType::Rust(ty),
        ]),
    );
    g.start = start;
    let control = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        0,
        ValueType::Control,
    );
    let effect = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        1,
        ValueType::Effect,
    );
    let a = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        2,
        ValueType::Rust(ty),
    );
    let b = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        3,
        ValueType::Rust(ty),
    );
    (g, ty, boolean, control, effect, a, b)
}

#[test]
fn checked_u64_uses_one_product_and_one_guarded_sum() {
    for guarded in [false, true] {
        let product = checked("mul", Opcode::Mul, guarded, false, IntegerWidth::W64);
        assert_eq!(
            product
                .instructions
                .iter()
                .filter(|i| matches!(i, MachineInstr::MulOverflow { .. }))
                .count(),
            1
        );
        assert_eq!(
            product
                .instructions
                .iter()
                .filter(|i| matches!(i, MachineInstr::IMul { .. }))
                .count(),
            0
        );
        product.emit_machine_code().unwrap();
    }
    let sum = checked("add", Opcode::Add, true, false, IntegerWidth::W64);
    assert_eq!(
        sum.instructions
            .iter()
            .filter(|i| matches!(i, MachineInstr::Add { .. }))
            .count(),
        1
    );
    for a in [0u64, 1, 10, u64::MAX / 10, u64::MAX] {
        for b in [0u64, 1, 10, u64::MAX] {
            let (value, overflow) = a.overflowing_add(b);
            let got = kfrc_x86::machine::eval::evaluate(&sum, &[a as i64, b as i64]).unwrap();
            assert_eq!(got as u64, if overflow { 37 } else { value });
        }
    }
}

#[test]
fn signed_and_narrow_products_keep_their_existing_canonicalization() {
    for (signed, width) in [
        (true, IntegerWidth::W64),
        (false, IntegerWidth::W8),
        (true, IntegerWidth::W8),
    ] {
        let product = checked("separate", Opcode::Mul, false, signed, width);
        assert_eq!(
            product
                .instructions
                .iter()
                .filter(|i| matches!(i, MachineInstr::IMul { .. }))
                .count(),
            1
        );
        product.emit_machine_code().unwrap();
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn control_flow(looping: bool) -> MachineFunction {
    let (mut g, ty, boolean, control, effect, a, b) = setup(
        if looping { "mul_loop" } else { "mul_siblings" },
        false,
        IntegerWidth::W64,
    );
    let zero = g.add_constant(0, ty);
    let one = g.add_constant(1, ty);
    let branch = |g: &mut FunctionGraph, control, condition| {
        let branch = g.add_node(
            Opcode::If,
            control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
        );
        let yes = g.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            0,
            ValueType::Control,
        );
        let no = g.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            1,
            ValueType::Control,
        );
        (yes, no)
    };
    if looping {
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
            ComparePredicate::UnsignedLess as u32,
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
            ArithmeticFlags::NUW,
            ValueType::Rust(ty),
        );
        let next = g.add_node(Opcode::Add, success, &[index, one], 0, ValueType::Rust(ty));
        let backedge = g.add_node(Opcode::Jump, success, &[], 0, ValueType::Control);
        g.set_input(region, 1, backedge);
        g.set_input(index, 2, next);
        g.set_input(accumulator, 2, product);
    } else {
        let less = g.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[a, b],
            ComparePredicate::UnsignedLess as u32,
            ValueType::Rust(boolean),
        );
        let (yes, no) = branch(&mut g, control, less);
        for (control, mask) in [(yes, zero), (no, one)] {
            let product = g.add_node(Opcode::Mul, control, &[a, b], 0, ValueType::Rust(ty));
            let overflow = g.add_node(
                Opcode::MulOverflow,
                control,
                &[a, b],
                0,
                ValueType::Rust(boolean),
            );
            let flag = g.add_node(
                Opcode::IntCast,
                NodeId::NONE,
                &[overflow],
                0,
                ValueType::Rust(ty),
            );
            let combined = g.add_node(
                Opcode::BitXor,
                NodeId::NONE,
                &[product, flag],
                0,
                ValueType::Rust(ty),
            );
            let result = g.add_node(
                Opcode::BitXor,
                NodeId::NONE,
                &[combined, mask],
                0,
                ValueType::Rust(ty),
            );
            g.add_node(
                Opcode::Return,
                control,
                &[effect, result],
                0,
                ValueType::Control,
            );
        }
    }
    verify_graph(&mut g).unwrap();
    lower_layout(&mut g).unwrap();
    verify_graph(&mut g).unwrap();
    let function = lower_function(&g).unwrap();
    assert_eq!(function.loop_headers.len(), usize::from(looping));
    function
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn encoded_checked_results_match_native_boundary_and_random_inputs() {
    use kfrc_x86::{FunctionCode, SymbolBinding, write_object};
    use std::process::Command;
    let mut functions = Vec::new();
    for (name, op, guarded) in [
        ("mul_both", Opcode::Mul, false),
        ("mul_guard", Opcode::Mul, true),
        ("add_both", Opcode::Add, false),
        ("add_guard", Opcode::Add, true),
    ] {
        functions.push(FunctionCode {
            function: checked(name, op, guarded, false, IntegerWidth::W64),
            binding: SymbolBinding::Global { hidden: false },
        });
    }
    for looping in [false, true] {
        functions.push(FunctionCode {
            function: control_flow(looping),
            binding: SymbolBinding::Global { hidden: false },
        });
    }
    let dir = std::env::temp_dir().join(format!(
        "kfrc-checked-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("checked.o"), write_object(&functions).unwrap()).unwrap();
    std::fs::write(dir.join("driver.c"),r#"
#include <stdint.h>
#include <stdio.h>
extern uint64_t mul_both(uint64_t,uint64_t),mul_guard(uint64_t,uint64_t),add_both(uint64_t,uint64_t),add_guard(uint64_t,uint64_t);
extern uint64_t mul_loop(uint64_t,uint64_t),mul_siblings(uint64_t,uint64_t);
static int check(uint64_t a,uint64_t b) {
  uint64_t m,s; unsigned mo=__builtin_mul_overflow(a,b,&m),so=__builtin_add_overflow(a,b,&s);
  return mul_both(a,b)!=(m^mo) || mul_guard(a,b)!=(mo?37:m) || add_both(a,b)!=(s^so) || add_guard(a,b)!=(so?37:s) || mul_siblings(a,b)!=(m^mo^(a<b?0:1));
}
int main(void) {
  uint64_t values[]={0,1,2,9,10,UINT32_MAX,UINT64_MAX/10,UINT64_MAX/10+1,UINT64_C(1)<<63,UINT64_MAX-1,UINT64_MAX};
  for(unsigned i=0;i<sizeof(values)/sizeof(*values);i++) for(unsigned j=0;j<sizeof(values)/sizeof(*values);j++) if(check(values[i],values[j])) return 1;
  for(unsigned i=0;i<sizeof(values)/sizeof(*values);i++) for(unsigned n=0;n<24;n++) {
    uint64_t expected=values[i];
    for(unsigned j=0;j<n;j++) if(__builtin_mul_overflow(expected,UINT64_C(10),&expected)) {expected=37;break;}
    if(mul_loop(n,values[i])!=expected) return 3;
  }
  uint64_t state=UINT64_C(0x123456789abcdef0);
  for(unsigned i=0;i<10000;i++) { state^=state<<13;state^=state>>7;state^=state<<17; if(check(state,i%37) || check(state,state>>3)) return 2; }
  puts("checked value/overflow PASS");return 0;
}
"#).unwrap();
    let output = Command::new("cc")
        .current_dir(&dir)
        .args(["driver.c", "checked.o", "-o", "driver"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(dir.join("driver")).output().unwrap();
    assert!(
        output.status.success(),
        "status {:?}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout)
    );
}
