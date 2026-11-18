use super::*;
use cranelift_codegen::Context;
use cranelift_codegen::control::ControlPlane;
use cranelift_codegen::ir::{AbiParam, InstBuilder, Type, condcodes::IntCC, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

fn compile(word: Type, bits: i64, reversed: bool) -> String {
    let machine = Machine::new(MachineOptions {
        verify: true,
        ..MachineOptions::default()
    })
    .unwrap();
    let mut context = Context::new();
    context.func.signature.call_conv = machine.isa.default_call_conv();
    context.func.signature.params.push(AbiParam::new(word));
    context
        .func
        .signature
        .returns
        .push(AbiParam::new(types::I8));
    let mut frontend = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut context.func, &mut frontend);
    let block = builder.create_block();
    builder.append_block_params_for_function_params(block);
    builder.switch_to_block(block);
    let index = builder.block_params(block)[0];
    let one = builder.ins().iconst(word, 1);
    let shifted = builder.ins().ishl(one, index);
    let bitmap = builder.ins().iconst(word, bits);
    let (a, b) = if reversed {
        (shifted, bitmap)
    } else {
        (bitmap, shifted)
    };
    let selected = builder.ins().band(a, b);
    let contains = builder.ins().icmp_imm_u(IntCC::NotEqual, selected, 0);
    builder.ins().return_(&[contains]);
    builder.seal_all_blocks();
    builder.finalize(machine.isa.frontend_config());
    cranelift_codegen::verify_function(&context.func, &*machine.isa).unwrap();
    context.set_disasm(true);
    context
        .compile(&*machine.isa, &mut ControlPlane::default())
        .unwrap();
    context.compiled_code().unwrap().vcode.clone().unwrap()
}

#[test]
fn canonical_bitmap_test_uses_a_register_bt() {
    for (word, bits) in [(types::I32, 0x86), (types::I64, i64::MIN | 0x284)] {
        for reversed in [false, true] {
            let text = compile(word, bits, reversed);
            assert!(
                text.lines().any(|line| line.trim_start().starts_with("bt ")
                    || line.trim_start().starts_with("btl ")
                    || line.trim_start().starts_with("btq ")),
                "{text}"
            );
        }
    }
}
