//! The Rust SoN frontend does not yet accept floats. Exercise their cold call
//! register types directly in CLIF, including SIMD and stack arguments.
use super::*;
use cranelift_codegen::ir::{AbiParam, InstBuilder, types};
use cranelift_codegen::{Context, control::ControlPlane};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

#[test]
fn cold_call_register_copies_preserve_float_vector_and_integer_types() {
    for optimization in [Optimization::None, Optimization::Speed] {
        let machine = Machine::new(MachineOptions {
            optimization,
            verify: true,
            ..MachineOptions::default()
        })
        .unwrap();
        let mut context = Context::new();
        context.func.signature.call_conv = machine.isa.default_call_conv();
        let params = [types::I64, types::F32, types::F64, types::I8X16];
        context
            .func
            .signature
            .params
            .extend(params.map(AbiParam::new));
        context
            .func
            .signature
            .returns
            .push(AbiParam::new(types::I64));
        let mut frontend = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut frontend);
        let entry = builder.create_block();
        let cold = builder.create_block();
        let hot = builder.create_block();
        builder.set_cold_block(cold);
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        let args = builder.block_params(entry).to_vec();
        builder.ins().brif(args[0], cold, &[], hot, &[]);
        builder.switch_to_block(hot);
        builder.ins().return_(&[args[0]]);
        builder.switch_to_block(cold);
        let mut signature = builder.func.signature.clone();
        signature.params = vec![AbiParam::new(types::I64); 7];
        signature
            .params
            .extend(params[1..].iter().copied().map(AbiParam::new));
        let signature = builder.import_signature(signature);
        let mut call_args = vec![args[0]; 7];
        call_args.extend_from_slice(&args[1..]);
        let call = builder.ins().call_indirect(signature, args[0], &call_args);
        let result = builder.inst_results(call)[0];
        builder.ins().return_(&[result]);
        builder.seal_all_blocks();
        builder.finalize(machine.isa.frontend_config());
        cranelift_codegen::verify_function(&context.func, &*machine.isa).unwrap();
        // MachineOptions::verify also enables regalloc's checker. It validates
        // the post-copy argument constraints, not only the original CLIF.
        context
            .compile(&*machine.isa, &mut ControlPlane::default())
            .unwrap();
    }
}
