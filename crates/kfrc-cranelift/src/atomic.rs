//! Small target runtime routines for atomic operations. Keeping the call opaque
//! in the semantic IR gives every operation a conservative read/write effect.
//! Cranelift atomics are sequentially consistent; weaker Rust orderings may
//! safely use that stronger ordering. No ordinary load/store substitutes here.

use cranelift_codegen::Context;
use cranelift_codegen::ir::{AbiParam, AtomicRmwOp, InstBuilder, MemFlagsData, Signature, types};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use kfrc_body::EncodedFunction;

use crate::{LowerError, Machine};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtomicOperation {
    Load,
    Store,
    Sub,
    Add,
    Exchange,
    CompareExchange,
    Fence,
}

impl AtomicOperation {
    pub fn name(self) -> &'static str {
        match self {
            Self::Load => "load",
            Self::Store => "store",
            Self::Sub => "sub",
            Self::Add => "add",
            Self::Exchange => "exchange",
            Self::CompareExchange => "compare_exchange",
            Self::Fence => "fence",
        }
    }
}

pub fn lower_atomic_runtime(
    symbol: &str,
    operation: AtomicOperation,
    bits: u8,
    signed: bool,
    machine: &Machine,
) -> Result<EncodedFunction, LowerError> {
    let ty = match bits {
        8 => types::I8,
        16 => types::I16,
        32 => types::I32,
        64 => types::I64,
        _ => {
            return Err(LowerError::Unsupported(format!(
                "{bits}-bit atomic runtime"
            )));
        }
    };
    let lane = || {
        let parameter = AbiParam::new(ty);
        if bits >= 32 {
            parameter
        } else if signed {
            parameter.sext()
        } else {
            parameter.uext()
        }
    };
    let mut context = Context::new();
    context.func.signature = Signature::new(CallConv::SystemV);
    if operation != AtomicOperation::Fence {
        context
            .func
            .signature
            .params
            .push(AbiParam::new(types::I64));
        if operation != AtomicOperation::Load {
            context.func.signature.params.push(lane());
        }
        if operation == AtomicOperation::CompareExchange {
            context.func.signature.params.push(lane());
        }
        if operation != AtomicOperation::Store {
            context.func.signature.returns.push(lane());
        }
        if operation == AtomicOperation::CompareExchange {
            context
                .func
                .signature
                .returns
                .push(AbiParam::new(types::I8).uext());
        }
    }
    let mut builder_context = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
    let block = builder.create_block();
    builder.append_block_params_for_function_params(block);
    builder.switch_to_block(block);
    let parameters = builder.block_params(block).to_vec();
    let mut success = None;
    let result = match operation {
        AtomicOperation::Load => Some(builder.ins().atomic_load(
            ty,
            MemFlagsData::trusted(),
            parameters[0],
        )),
        AtomicOperation::Store => {
            builder
                .ins()
                .atomic_store(MemFlagsData::trusted(), parameters[1], parameters[0]);
            None
        }
        AtomicOperation::Sub | AtomicOperation::Add | AtomicOperation::Exchange => {
            Some(builder.ins().atomic_rmw(
                ty,
                MemFlagsData::trusted(),
                match operation {
                    AtomicOperation::Sub => AtomicRmwOp::Sub,
                    AtomicOperation::Add => AtomicRmwOp::Add,
                    AtomicOperation::Exchange => AtomicRmwOp::Xchg,
                    _ => unreachable!(),
                },
                parameters[0],
                parameters[1],
            ))
        }
        AtomicOperation::CompareExchange => {
            let old = builder.ins().atomic_cas(
                MemFlagsData::trusted(),
                parameters[0],
                parameters[1],
                parameters[2],
            );
            success = Some(builder.ins().icmp(
                cranelift_codegen::ir::condcodes::IntCC::Equal,
                old,
                parameters[1],
            ));
            Some(old)
        }
        AtomicOperation::Fence => {
            builder.ins().fence();
            None
        }
    };
    builder
        .ins()
        .return_(&result.into_iter().chain(success).collect::<Vec<_>>());
    builder.seal_all_blocks();
    builder.finalize(machine.isa.frontend_config());
    cranelift_codegen::verify_function(&context.func, &*machine.isa)
        .map_err(|errors| LowerError::Malformed(format!("atomic runtime: {errors}")))?;
    crate::emit::encode(symbol, &mut context, &[], machine)
}
