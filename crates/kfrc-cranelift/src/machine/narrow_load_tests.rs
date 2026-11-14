//! Exercise instruction selection directly, without mid-end folding hiding
//! the shared-load case or the high-bit counterexamples.
use super::*;
use cranelift_codegen::Context;
use cranelift_codegen::control::ControlPlane;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlagsData, Type, types};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

#[derive(Clone, Copy)]
enum Source {
    Load,
    UnsignedLoad,
    SignedLoad,
    Increment,
    Parameter,
}

fn compile(source: Source, narrow: Type, target: Type, shared: bool) -> String {
    let machine = Machine::new(MachineOptions {
        optimization: Optimization::None,
        verify: true,
        ..MachineOptions::default()
    })
    .unwrap();
    let mut context = Context::new();
    context.func.signature.call_conv = machine.isa.default_call_conv();
    context
        .func
        .signature
        .params
        .push(AbiParam::new(types::I64));
    context.func.signature.params.push(AbiParam::new(narrow));
    context.func.signature.returns.push(AbiParam::new(target));
    let mut frontend = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut context.func, &mut frontend);
    let block = builder.create_block();
    builder.append_block_params_for_function_params(block);
    builder.switch_to_block(block);
    let address = builder.block_params(block)[0];
    let argument = builder.block_params(block)[1];
    let flags = MemFlagsData::new();
    let value = match source {
        Source::Load | Source::Increment => builder.ins().load(narrow, flags, address, 0),
        Source::UnsignedLoad => builder.ins().uload8(narrow, flags, address, 0),
        Source::SignedLoad => builder.ins().sload8(narrow, flags, address, 0),
        Source::Parameter => argument,
    };
    let value = if matches!(source, Source::Increment) {
        builder.ins().iadd_imm_u(value, 1)
    } else {
        value
    };
    if shared {
        // A second observation prevents folding the load into the extension.
        builder.ins().store(flags, value, address, 8);
    }
    let result = builder.ins().uextend(target, value);
    builder.ins().return_(&[result]);
    builder.seal_all_blocks();
    builder.finalize(machine.isa.frontend_config());
    cranelift_codegen::verify_function(&context.func, &*machine.isa).unwrap();
    context.set_disasm(true);
    context
        .compile(&*machine.isa, &mut ControlPlane::default())
        .unwrap();
    context.compiled_code().unwrap().vcode.clone().unwrap()
}

fn register_zero_extensions(text: &str) -> usize {
    text.lines()
        .filter(|line| line.contains("movz") && !line.contains('('))
        .count()
}

#[test]
fn shared_unsigned_loads_keep_one_memory_extension() {
    for narrow in [types::I8, types::I16, types::I32] {
        for target in [types::I32, types::I64] {
            if narrow.bits() >= target.bits() {
                continue;
            }
            for shared in [false, true] {
                let text = compile(Source::Load, narrow, target, shared);
                assert_eq!(register_zero_extensions(&text), 0, "{text}");
            }
        }
    }
    let text = compile(Source::UnsignedLoad, types::I16, types::I64, true);
    assert_eq!(register_zero_extensions(&text), 0, "{text}");
}

#[test]
fn other_narrow_producers_keep_their_required_extension() {
    for (source, narrow) in [
        (Source::SignedLoad, types::I16),
        (Source::Increment, types::I8),
        (Source::Increment, types::I16),
        (Source::Parameter, types::I8),
        (Source::Parameter, types::I16),
    ] {
        let text = compile(source, narrow, types::I64, true);
        assert!(register_zero_extensions(&text) > 0, "{text}");
    }
}
