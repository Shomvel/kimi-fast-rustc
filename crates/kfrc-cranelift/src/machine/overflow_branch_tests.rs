use super::*;
use cranelift_codegen::Context;
use cranelift_codegen::control::ControlPlane;
use cranelift_codegen::ir::{AbiParam, BlockArg, InstBuilder, MemFlagsData, Type, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

#[derive(Clone, Copy)]
enum Shape {
    Adjacent,
    FlagAsEdgeArgument,
    InterveningStore,
}

fn compile(ty: Type, operation: usize, shape: Shape) -> String {
    let machine = Machine::new(MachineOptions {
        optimization: Optimization::None,
        verify: true,
        ..MachineOptions::default()
    })
    .unwrap();
    let mut context = Context::new();
    context.func.signature.call_conv = machine.isa.default_call_conv();
    context.func.signature.params = [ty, ty, types::I64].map(AbiParam::new).to_vec();
    context.func.signature.returns.push(AbiParam::new(ty));
    let mut frontend = FunctionBuilderContext::new();
    let mut b = FunctionBuilder::new(&mut context.func, &mut frontend);
    let entry = b.create_block();
    let taken = b.create_block();
    let other = b.create_block();
    b.append_block_params_for_function_params(entry);
    let taken_result = b.append_block_param(taken, ty);
    let other_result = b.append_block_param(other, ty);
    let taken_flag =
        matches!(shape, Shape::FlagAsEdgeArgument).then(|| b.append_block_param(taken, types::I8));
    b.switch_to_block(entry);
    let [x, y, addr] = *b.block_params(entry) else {
        unreachable!()
    };
    let (result, flag) = match operation {
        0 => b.ins().uadd_overflow(x, y),
        1 => b.ins().sadd_overflow(x, y),
        2 => b.ins().usub_overflow(x, y),
        3 => b.ins().ssub_overflow(x, y),
        4 => b.ins().umul_overflow(x, y),
        5 => b.ins().smul_overflow(x, y),
        _ => unreachable!(),
    };
    if matches!(shape, Shape::InterveningStore) {
        b.ins().store(MemFlagsData::new(), x, addr, 0);
    }
    let taken_args = [BlockArg::Value(result), BlockArg::Value(flag)];
    b.ins().brif(
        flag,
        taken,
        &taken_args[..1 + usize::from(taken_flag.is_some())],
        other,
        &[BlockArg::Value(result)],
    );
    b.switch_to_block(taken);
    b.ins().store(MemFlagsData::new(), taken_result, addr, 0);
    if let Some(taken_flag) = taken_flag {
        b.ins().store(MemFlagsData::new(), taken_flag, addr, 16);
    }
    b.ins().return_(&[taken_result]);
    b.switch_to_block(other);
    let inverted = b.ins().bnot(other_result);
    b.ins().return_(&[inverted]);
    b.seal_all_blocks();
    b.finalize(machine.isa.frontend_config());
    cranelift_codegen::verify_function(&context.func, &*machine.isa).unwrap();
    context.set_disasm(true);
    context
        .compile(&*machine.isa, &mut ControlPlane::default())
        .unwrap();
    context.compiled_code().unwrap().vcode.clone().unwrap()
}

#[test]
fn overflow_branch_keeps_the_arithmetic_result_on_both_edges() {
    let enabled = std::env::var_os("KFRC_CLIF_OVERFLOW_BRANCH").is_some_and(|v| v == "1");
    for ty in [types::I8, types::I16, types::I32, types::I64, types::I128] {
        let operations = if ty == types::I128 { 4 } else { 6 };
        for operation in 0..operations {
            for shape in [
                Shape::Adjacent,
                Shape::FlagAsEdgeArgument,
                Shape::InterveningStore,
            ] {
                let code = compile(ty, operation, shape);
                let materialized = code
                    .lines()
                    .any(|line| line.trim_start().starts_with("set"));
                assert_eq!(
                    materialized,
                    !(enabled && ty != types::I128 && matches!(shape, Shape::Adjacent)),
                    "{ty}, op {operation}: {code}"
                );
            }
        }
    }
}
