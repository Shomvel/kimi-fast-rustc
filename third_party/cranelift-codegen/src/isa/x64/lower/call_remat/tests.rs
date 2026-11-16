use super::*;
use crate::cursor::{Cursor, FuncCursor};
use crate::dominator_tree::DominatorTree;
use crate::flowgraph::ControlFlowGraph;
use crate::ir::{
    AbiParam, ArgumentExtension, ExtFuncData, ExternalName, Function, InstBuilder, Signature, Type,
    condcodes::IntCC,
};
use crate::isa::CallConv;
use crate::isa::x64::{EmitInfo, X64Backend, abi, settings as x64_settings};
use crate::machinst::{ABIArgSlot, BlockLoweringOrder, SigSet};
use crate::settings::{self, Configurable};
use alloc::vec::Vec;
use target_lexicon::triple;

fn backend() -> X64Backend {
    let mut settings = settings::builder();
    settings.set("opt_level", "speed").unwrap();
    settings.set("regalloc_checker", "true").unwrap();
    let flags = settings::Flags::new(settings);
    let isa_flags = x64_settings::Flags::new(&flags, &x64_settings::builder());
    X64Backend::new_with_flags(triple!("x86_64-unknown-linux-gnu"), flags, isa_flags).unwrap()
}

fn fixture(argument_count: usize) -> (Function, crate::ir::SigRef, crate::ir::Inst) {
    let mut function =
        Function::with_name_signature(Default::default(), Signature::new(CallConv::SystemV));
    let mut sig = Signature::new(CallConv::SystemV);
    sig.params = vec![AbiParam::new(types::I64); argument_count];
    // Multi-result returns use the original result-lowering path.
    sig.returns = vec![AbiParam::new(types::I64), AbiParam::new(types::I64)];
    let sig = function.import_signature(sig);
    let callee = function.import_function(ExtFuncData {
        name: ExternalName::testcase("callee"),
        signature: sig,
        colocated: true,
        patchable: false,
    });
    let entry = function.dfg.make_block();
    let mut cursor = FuncCursor::new(&mut function);
    cursor.insert_block(entry);
    let six = cursor.ins().iconst(types::I64, 6);
    let arguments = vec![six; argument_count];
    let call = cursor.ins().call(callee, &arguments);
    cursor.ins().return_(&[]);
    (function, sig, call)
}

fn lower<'a>(function: &'a Function, backend: &X64Backend) -> Lower<'a, Inst> {
    let cfg = ControlFlowGraph::with_function(function);
    let domtree = DominatorTree::with_function(function, &cfg);
    let sigs = SigSet::new::<abi::X64ABIMachineSpec>(function, &backend.flags).unwrap();
    let abi = abi::X64Callee::new(function, backend, &backend.x64_flags, &sigs).unwrap();
    let emit_info = EmitInfo::new(backend.flags.clone(), backend.x64_flags.clone());
    let order = BlockLoweringOrder::new(function, &domtree, &mut Default::default());
    Lower::new(function, abi, emit_info, order, sigs, backend.flags.clone()).unwrap()
}

fn slot(ty: Type, extension: ArgumentExtension) -> ABIArg {
    ABIArg::stack(0, ty, extension, ArgumentPurpose::Normal)
}

#[test]
fn scalar_iconst_bits_preserve_abi_extension_and_stack_arguments() {
    let mut function = Function::new();
    let block = function.dfg.make_block();
    let mut cursor = FuncCursor::new(&mut function);
    cursor.insert_block(block);
    let mut cases = Vec::new();
    for ty in [types::I8, types::I16, types::I32, types::I64] {
        for input in [0, 6, -1, i64::MIN, i64::MAX] {
            let value = cursor.ins().iconst(ty, input);
            let expected = if ty.bits() < 64 {
                (input as u64) & ((1u64 << ty.bits()) - 1)
            } else {
                input as u64
            };
            cases.push((value, ty, expected));
        }
    }
    cursor.ins().return_(&[]);
    for (value, ty, expected) in cases {
        for extension in [
            ArgumentExtension::None,
            ArgumentExtension::Uext,
            ArgumentExtension::Sext,
        ] {
            let (size, bits) = constant(&function.dfg, value, &slot(ty, extension)).unwrap();
            assert_eq!(bits, expected);
            assert!(
                size == if ty == types::I64 {
                    OperandSize::Size64
                } else {
                    OperandSize::Size32
                }
            );
        }
    }
}

#[test]
fn non_scalar_and_special_abi_values_keep_the_original_path() {
    let mut function = Function::new();
    let block = function.dfg.make_block();
    let parameter = function.dfg.append_block_param(block, types::I64);
    let mut cursor = FuncCursor::new(&mut function);
    cursor.insert_block(block);
    let scalar = cursor.ins().iconst(types::I64, 6);
    let wide = cursor.ins().iconst(types::I128, 6);
    let float = cursor.ins().f64const(6.0);
    let byte = cursor.ins().iconst(types::I8, 6);
    let vector = cursor.ins().splat(types::I8X16, byte);
    cursor.ins().return_(&[]);
    for (value, ty) in [
        (parameter, types::I64),
        (wide, types::I128),
        (float, types::F64),
        (vector, types::I8X16),
    ] {
        assert!(constant(&function.dfg, value, &slot(ty, ArgumentExtension::None)).is_none());
    }
    let special = ABIArg::stack(
        0,
        types::I64,
        ArgumentExtension::None,
        ArgumentPurpose::VMContext,
    );
    assert!(constant(&function.dfg, scalar, &special).is_none());
    let stack_slot = ABIArgSlot::Stack {
        offset: 0,
        ty: types::I64,
        extension: ArgumentExtension::None,
    };
    for abi in [
        ABIArg::Slots {
            slots: smallvec::smallvec![],
            purpose: ArgumentPurpose::Normal,
        },
        ABIArg::Slots {
            slots: smallvec::smallvec![stack_slot, stack_slot],
            purpose: ArgumentPurpose::Normal,
        },
        ABIArg::StructArg {
            offset: 0,
            size: 8,
            purpose: ArgumentPurpose::Normal,
        },
        ABIArg::ImplicitPtrArg {
            pointer: stack_slot,
            offset: 0,
            ty: types::I64,
            purpose: ArgumentPurpose::Normal,
        },
    ] {
        assert!(constant(&function.dfg, scalar, &abi).is_none());
    }
    assert!(
        constant(
            &function.dfg,
            scalar,
            &slot(types::I32, ArgumentExtension::None)
        )
        .is_none()
    );
}

#[test]
fn direct_call_copies_are_fresh_and_have_call_and_function_caps() {
    let (function, signature, call) = fixture(8);
    let backend = backend();
    let mut lower = lower(&function, &backend);
    let sig = lower
        .sigs()
        .abi_sig_for_signature(&function.dfg.signatures[signature]);
    let args = match function.dfg.insts[call] {
        InstructionData::Call { args, .. } => args,
        _ => unreachable!(),
    };
    let original = lower.put_value_in_regs(function.dfg.inst_args(call)[0]);
    for _ in 0..16 {
        let copies = lower_args(&mut lower, sig, false, (args, 0), true);
        assert_eq!(copies.len(), 8);
        let fresh: Vec<_> = copies
            .iter()
            .take(4)
            .map(|r| r.only_reg().unwrap())
            .collect();
        for (index, reg) in fresh.iter().enumerate() {
            assert_ne!(Some(*reg), original.only_reg());
            assert!(!fresh[..index].contains(reg));
        }
        assert!(
            copies
                .iter()
                .skip(4)
                .all(|regs| regs.only_reg() == original.only_reg())
        );
    }
    assert_eq!(lower.x64_call_constant_remats, MAX_PER_FUNCTION);
    assert!(
        lower_args(&mut lower, sig, false, (args, 0), true)
            .iter()
            .all(|regs| regs.only_reg() == original.only_reg())
    );
}

#[test]
fn disabled_and_patchable_calls_do_not_allocate_replacement_registers() {
    let (function, signature, call) = fixture(2);
    let backend = backend();
    let mut lower = lower(&function, &backend);
    let sig = lower
        .sigs()
        .abi_sig_for_signature(&function.dfg.signatures[signature]);
    let args = match function.dfg.insts[call] {
        InstructionData::Call { args, .. } => args,
        _ => unreachable!(),
    };
    let original = lower.put_value_in_regs(function.dfg.inst_args(call)[0]);
    for (enabled, patchable) in [(false, false), (false, true), (true, true)] {
        let registers = lower_args(&mut lower, sig, patchable, (args, 0), enabled);
        assert!(
            registers
                .iter()
                .all(|regs| regs.only_reg() == original.only_reg())
        );
        assert_eq!(lower.x64_call_constant_remats, 0);
    }
}

#[test]
fn production_call_lowering_reaches_vcode_and_allocation() {
    for colocated in [true, false] {
        let (mut function, _, call) = fixture(8);
        let func_ref = match function.dfg.insts[call] {
            InstructionData::Call { func_ref, .. } => func_ref,
            _ => unreachable!(),
        };
        function.dfg.ext_funcs[func_ref].colocated = colocated;
        let cfg = ControlFlowGraph::with_function(&function);
        let domtree = DominatorTree::with_function(&function, &cfg);
        let backend = backend();
        backend
            .compile_vcode(&function, &domtree, &mut Default::default())
            .unwrap();
    }
}

// This is the observed shape: a constant in a hot block is used by the
// condition and by a call in a cold successor. Its non-call use must remain
// intact while the call gets a local definition.
#[test]
fn cold_call_materializes_in_its_own_vcode_block() {
    let mut signature = Signature::new(CallConv::SystemV);
    signature.params.push(AbiParam::new(types::I64));
    signature.returns.push(AbiParam::new(types::I64));
    let mut function = Function::with_name_signature(Default::default(), signature.clone());
    signature.params.push(AbiParam::new(types::I64));
    let sig = function.import_signature(signature);
    let callee = function.import_function(ExtFuncData {
        name: ExternalName::testcase("cold_callee"),
        signature: sig,
        colocated: true,
        patchable: false,
    });
    let entry = function.dfg.make_block();
    let input = function.dfg.append_block_param(entry, types::I64);
    let hot = function.dfg.make_block();
    let cold = function.dfg.make_block();
    let mut cursor = FuncCursor::new(&mut function);
    cursor.insert_block(entry);
    let six = cursor.ins().iconst(types::I64, 6);
    let condition = cursor.ins().icmp(IntCC::UnsignedLessThan, input, six);
    cursor.ins().brif(condition, hot, &[], cold, &[]);
    cursor.insert_block(hot);
    cursor.ins().return_(&[input]);
    cursor.insert_block(cold);
    let call = cursor.ins().call(callee, &[six, input]);
    let result = cursor.func.dfg.inst_results(call)[0];
    cursor.ins().return_(&[result]);
    function.layout.set_cold(cold);

    let backend = backend();
    let lowered = lower(&function, &backend)
        .lower(&backend, &mut Default::default())
        .unwrap();
    let text = format!("{lowered:?}");
    let cold_marker = format!("(original IR block: {cold})");
    let cold_body = text
        .split_once(&cold_marker)
        .unwrap()
        .1
        .split("\nBlock ")
        .next()
        .unwrap();
    let cold_constant = cold_body
        .lines()
        .any(|line| line.contains("movl") && line.contains("$0x6"));
    assert_eq!(cold_constant, enabled(), "{text}");

    // Exercise the production allocator and its enabled checker as well.
    let cfg = ControlFlowGraph::with_function(&function);
    let domtree = DominatorTree::with_function(&function, &cfg);
    let (vcode, allocation) = backend
        .compile_vcode(&function, &domtree, &mut Default::default())
        .unwrap();
    #[cfg(feature = "std")]
    std::println!(
        "R28 enabled={} CLIF:\n{}\nPREALLOCATION:\n{text}\nVCODE RETURNED WITH ALLOCATION:\n{vcode:?}\nEDITS:\n{:?}",
        enabled(),
        function.display(),
        allocation.edits
    );
    #[cfg(not(feature = "std"))]
    let _ = (vcode, allocation);
}
