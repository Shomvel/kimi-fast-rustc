//! Loop rotation (vendored Cranelift patch 9) changes block layout only.
use super::*;
use cranelift_codegen::ir::{AbiParam, InstBuilder, condcodes::IntCC, types};
use cranelift_codegen::{Context, control::ControlPlane};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

/// `fn(n) { let (mut i, mut s) = (0, 0); while i < n { s += i; i += 1 } s }`
fn compile_counting_loop() -> (String, Vec<u8>) {
    let machine = Machine::new(MachineOptions {
        optimization: Optimization::None,
        verify: true,
        ..MachineOptions::default()
    })
    .unwrap();
    let mut context = Context::new();
    context.func.signature.call_conv = machine.isa.default_call_conv();
    context.func.signature.params.push(AbiParam::new(types::I64));
    context.func.signature.returns.push(AbiParam::new(types::I64));
    let mut frontend = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut context.func, &mut frontend);
    let entry = builder.create_block();
    let header = builder.create_block();
    let body = builder.create_block();
    let exit = builder.create_block();
    builder.append_block_params_for_function_params(entry);
    builder.append_block_param(header, types::I64);
    builder.append_block_param(header, types::I64);
    builder.append_block_param(exit, types::I64);
    builder.switch_to_block(entry);
    let n = builder.block_params(entry)[0];
    let zero = builder.ins().iconst(types::I64, 0);
    builder.ins().jump(header, &[zero.into(), zero.into()]);
    builder.switch_to_block(header);
    let (i, s) = (builder.block_params(header)[0], builder.block_params(header)[1]);
    let more = builder.ins().icmp(IntCC::UnsignedLessThan, i, n);
    builder.ins().brif(more, body, &[], exit, &[s.into()]);
    builder.switch_to_block(body);
    let s2 = builder.ins().iadd(s, i);
    let i2 = builder.ins().iadd_imm(i, 1);
    builder.ins().jump(header, &[i2.into(), s2.into()]);
    builder.switch_to_block(exit);
    let result = builder.block_params(exit)[0];
    builder.ins().return_(&[result]);
    builder.seal_all_blocks();
    builder.finalize(machine.isa.frontend_config());
    cranelift_codegen::verify_function(&context.func, &*machine.isa).unwrap();
    context.set_disasm(true);
    context
        .compile(&*machine.isa, &mut ControlPlane::default())
        .unwrap();
    let code = context.compiled_code().unwrap();
    (code.vcode.clone().unwrap(), code.buffer.data().to_vec())
}

#[test]
fn counting_loop_header_is_emitted_after_its_latch() {
    let (disasm, code) = compile_counting_loop();
    if std::env::var_os("KFRC_CLIF_LOOP_ROTATION").is_some_and(|value| value == "0") {
        return;
    }
    // In emission order the body (the address-form adds) precedes the header's compare,
    // and the only unconditional jump is the one entering the loop.
    let add = disasm.find("lea").expect(&disasm);
    let compare = disasm.find("cmp").expect(&disasm);
    assert!(add < compare, "header not rotated:\n{disasm}");
    let short_or_near_jumps = code.windows(1).filter(|byte| byte[0] == 0xeb).count();
    assert!(short_or_near_jumps <= 1, "{disasm}\n{code:02x?}");
}
