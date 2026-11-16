//! Direct CLIF coverage keeps ABI extensions visible to instruction selection.
use super::*;
use cranelift_codegen::ir::{AbiParam, InstBuilder, Type, types};
use cranelift_codegen::{Context, control::ControlPlane};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};

fn compile(
    ty: Type,
    signed: Option<bool>,
    constant: Option<u64>,
    shared: bool,
) -> (String, Vec<u8>) {
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
    context.func.signature.params.push(AbiParam::new(ty));
    let result = AbiParam::new(ty);
    context.func.signature.returns.push(match signed {
        Some(true) => result.sext(),
        Some(false) => result.uext(),
        None => result,
    });
    let mut frontend = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(&mut context.func, &mut frontend);
    let block = builder.create_block();
    builder.append_block_params_for_function_params(block);
    builder.switch_to_block(block);
    let address = builder.block_params(block)[0];
    let value = match constant {
        Some(value) => builder.ins().iconst(ty, value as i64),
        None => builder.block_params(block)[1],
    };
    if shared {
        builder.ins().store(
            cranelift_codegen::ir::MemFlagsData::new(),
            value,
            address,
            0,
        );
    }
    builder.ins().return_(&[value]);
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

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
fn with_native<T>(code: &[u8], run: impl FnOnce(*mut std::ffi::c_void) -> T) -> T {
    use std::ffi::c_void;
    unsafe extern "C" {
        fn mmap(
            address: *mut c_void,
            size: usize,
            protection: i32,
            flags: i32,
            fd: i32,
            offset: i64,
        ) -> *mut c_void;
        fn mprotect(address: *mut c_void, size: usize, protection: i32) -> i32;
        fn munmap(address: *mut c_void, size: usize) -> i32;
    }
    assert!(code.len() <= 4096);
    // SAFETY: A private writable page receives verified, relocation-free x64
    // code. It becomes read/execute before the callback calls it with the
    // generated CLIF register ABI, then is unmapped after that call returns.
    unsafe {
        let page = mmap(std::ptr::null_mut(), 4096, 3, 0x22, -1, 0);
        assert_ne!(page as isize, -1);
        std::ptr::copy_nonoverlapping(code.as_ptr(), page.cast::<u8>(), code.len());
        assert_eq!(mprotect(page, 4096, 5), 0);
        let result = run(page);
        assert_eq!(munmap(page, 4096), 0);
        result
    }
}

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
fn execute(code: &[u8], value: u64, saved: &mut u64) -> u64 {
    with_native(code, |page| {
        // SAFETY: The verified function takes an address and an integer in
        // the SysV argument registers. Its only memory access is a byte/word/
        // dword store within `saved`; the result is inspected as raw RAX bits.
        unsafe {
            let function: unsafe extern "C" fn(*mut u64, u64) -> u64 = std::mem::transmute(page);
            function(saved, value)
        }
    })
}

#[test]
fn integer_constants_materialize_the_required_signed_or_unsigned_return_bits() {
    for ty in [types::I8, types::I16, types::I32] {
        let mask = u64::MAX >> (64 - ty.bits());
        let sign = 1u64 << (ty.bits() - 1);
        for signed in [false, true] {
            for value in [0, 1, sign - 1, sign, mask] {
                for shared in [false, true] {
                    let (text, _code) = compile(ty, Some(signed), Some(value), shared);
                    assert!(!text.contains("movs") && !text.contains("movz"), "{text}");
                    assert!(text.contains("ret"), "{text}");
                    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
                    {
                        // Check the entire return register, including the ABI's
                        // extension bits, with the original CLIF width oracle.
                        let expected = if signed && value & sign != 0 {
                            value | !mask
                        } else {
                            value
                        };
                        let mut saved = 0xa5a5_a5a5_a5a5_a5a5;
                        assert_eq!(
                            execute(&_code, 0x5a5a_5a5a_5a5a_5a5a, &mut saved),
                            expected,
                            "{text}"
                        );
                        assert_eq!(
                            saved,
                            if shared {
                                (0xa5a5_a5a5_a5a5_a5a5 & !mask) | value
                            } else {
                                0xa5a5_a5a5_a5a5_a5a5
                            },
                            "{text}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn dynamic_narrow_returns_keep_their_abi_extension() {
    for ty in [types::I8, types::I16] {
        for signed in [false, true] {
            let (text, _code) = compile(ty, Some(signed), None, false);
            assert!(
                text.contains(if signed { "movs" } else { "movz" }),
                "{text}"
            );
            #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
            for value in [0u64, 1, 127, 128, 255, 256, 32767, 32768, 65535, u64::MAX] {
                let mask = u64::MAX >> (64 - ty.bits());
                let value = value | 0xfedc_ba98_0000_0000;
                let expected = if signed && value & (1 << (ty.bits() - 1)) != 0 {
                    (value & mask) | !mask
                } else {
                    value & mask
                };
                assert_eq!(execute(&_code, value, &mut 0), expected, "{text}");
            }
        }
    }
}

#[test]
fn unextended_narrow_constants_define_exact_value_bits_and_preserve_shared_stores() {
    for ty in [types::I8, types::I16] {
        let mask = u64::MAX >> (64 - ty.bits());
        let sign = 1u64 << (ty.bits() - 1);
        for value in [0, 1, sign - 1, sign, mask] {
            for shared in [false, true] {
                let (text, code) = compile(ty, None, Some(value), shared);
                if value != 0 {
                    assert!(
                        text.contains(if ty == types::I8 { "movb" } else { "movw" }),
                        "{text}"
                    );
                }
                if !shared {
                    let expected_bytes = if value == 0 || ty == types::I8 { 3 } else { 5 };
                    assert_eq!(code.len(), expected_bytes, "{text}");
                }
                #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
                {
                    // Only low bits belong to this CLIF result. Observe the
                    // full register but do not assert its unspecified bits.
                    let sentinel = 0xa5a5_a5a5_a5a5_a5a5;
                    let mut saved = sentinel;
                    assert_eq!(execute(&code, u64::MAX, &mut saved) & mask, value, "{text}");
                    assert_eq!(
                        saved,
                        if shared {
                            (sentinel & !mask) | value
                        } else {
                            sentinel
                        },
                        "{text}"
                    );
                }
            }
        }
    }
}

#[test]
fn mixed_extension_pair_keeps_each_registers_own_return_contract() {
    for extended in [false, true] {
        let machine = Machine::new(MachineOptions {
            optimization: Optimization::None,
            verify: true,
            ..MachineOptions::default()
        })
        .unwrap();
        let mut context = Context::new();
        context.func.signature.call_conv = machine.isa.default_call_conv();
        context.func.signature.returns = vec![
            AbiParam::new(types::I8),
            if extended {
                AbiParam::new(types::I16).sext()
            } else {
                AbiParam::new(types::I16)
            },
        ];
        let mut frontend = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut frontend);
        let block = builder.create_block();
        builder.switch_to_block(block);
        let first = builder.ins().iconst(types::I8, 0xab);
        let second = builder.ins().iconst(types::I16, -2);
        builder.ins().return_(&[first, second]);
        builder.seal_all_blocks();
        builder.finalize(machine.isa.frontend_config());
        cranelift_codegen::verify_function(&context.func, &*machine.isa).unwrap();
        context.set_disasm(true);
        context
            .compile(&*machine.isa, &mut ControlPlane::default())
            .unwrap();
        let code = context.compiled_code().unwrap();
        let text = code.vcode.as_ref().unwrap();
        assert!(text.contains("movb"), "{text}");
        #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
        {
            #[repr(C)]
            struct RegisterBits {
                first: u64,
                second: u64,
            }
            let actual = with_native(code.buffer.data(), |page| {
                // SAFETY: This CLIF signature returns two integer registers.
                // The C record observes their raw RAX/RDX contents; assertions
                // below read only each lane's declared/extended result bits.
                unsafe {
                    let function: unsafe extern "C" fn() -> RegisterBits =
                        std::mem::transmute(page);
                    function()
                }
            });
            assert_eq!(actual.first & 0xff, 0xab, "{text}");
            assert_eq!(actual.second & 0xffff, 0xfffe, "{text}");
            if extended {
                assert_eq!(actual.second, (-2i64) as u64, "{text}");
            }
        }
    }
}
