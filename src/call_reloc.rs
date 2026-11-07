//! Minimal direct calls for straight-line x86_64 System V functions.
//!
//! A call function contains only zero-argument direct calls followed by return.
//! It reuses the aligned empty frame from `x86_encode` and emits one
//! `R_X86_64_PLT32` relocation with addend -4 for every `E8 rel32` call. Values
//! live in caller-saved registers across a call and argument passing are rejected;
//! this is not an implementation of the Rust ABI or a general calling convention.

use crate::elf_reloc::{
    ElfRelocError, ObjectSymbol, R_X86_64_PLT32, Relocation, write_object_with_relocations,
};
use crate::x86::{MachineFunction, MachineInstr, Reg};
use crate::x86_encode::{EncodeError, emit_machine_code};
use std::error::Error;
use std::fmt;

pub const CALL_INSTRUCTION_SIZE: u64 = 5;
pub const CALL_RELOCATION_ADDEND: i64 = -4;
const CALL_OPCODE: u8 = 0xe8;
const EPILOGUE_SIZE: usize = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectCall {
    pub target: String,
    pub argument_count: usize,
    pub live_caller_saved: Vec<Reg>,
}

impl DirectCall {
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            argument_count: 0,
            live_caller_saved: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallFunction {
    pub name: String,
    pub calls: Vec<DirectCall>,
}

impl CallFunction {
    pub fn new(name: impl Into<String>, calls: Vec<DirectCall>) -> Self {
        Self {
            name: name.into(),
            calls,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallSite {
    pub call_offset: u64,
    pub relocation_offset: u64,
    pub return_offset: u64,
    pub target: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodedCallFunction {
    pub text: Vec<u8>,
    pub call_sites: Vec<CallSite>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CallRelocError {
    Encode(EncodeError),
    Elf(ElfRelocError),
    UnsupportedArguments {
        call_index: usize,
        argument_count: usize,
    },
    CallerSavedLiveAcrossCall {
        call_index: usize,
        reg: Reg,
    },
    UnknownTarget {
        function_index: usize,
        call_index: usize,
        target: String,
    },
    CodeSizeOverflow,
    AllocationFailed,
    CallOffsetOutOfBounds {
        call_index: usize,
        call_offset: u64,
        text_size: u64,
    },
    UnexpectedCallOpcode {
        call_index: usize,
        call_offset: u64,
        actual: u8,
    },
    RelocationOffsetMismatch {
        call_index: usize,
        expected: u64,
        actual: u64,
    },
    ReturnOffsetMismatch {
        call_index: usize,
        expected: u64,
        actual: u64,
    },
    NonZeroRel32Placeholder {
        call_index: usize,
        relocation_offset: u64,
    },
    OverlappingCalls {
        call_index: usize,
        previous_return_offset: u64,
        call_offset: u64,
    },
}

impl fmt::Display for CallRelocError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encode(error) => error.fmt(formatter),
            Self::Elf(error) => error.fmt(formatter),
            Self::UnsupportedArguments {
                call_index,
                argument_count,
            } => write!(
                formatter,
                "call {call_index} has {argument_count} arguments; only zero-argument System V calls are supported"
            ),
            Self::CallerSavedLiveAcrossCall { call_index, reg } => write!(
                formatter,
                "call {call_index} requires caller-saved register {reg} to remain live; call-clobber preservation is unsupported"
            ),
            Self::UnknownTarget {
                function_index,
                call_index,
                target,
            } => write!(
                formatter,
                "call {call_index} in function {function_index} references unknown target {target:?}"
            ),
            Self::CodeSizeOverflow => formatter.write_str("call function size overflows usize"),
            Self::AllocationFailed => {
                formatter.write_str("could not allocate the call function or object")
            }
            Self::CallOffsetOutOfBounds {
                call_index,
                call_offset,
                text_size,
            } => write!(
                formatter,
                "call {call_index} at offset {call_offset} needs {CALL_INSTRUCTION_SIZE} bytes in .text of size {text_size}"
            ),
            Self::UnexpectedCallOpcode {
                call_index,
                call_offset,
                actual,
            } => write!(
                formatter,
                "call {call_index} at offset {call_offset} has opcode {actual:#04x}, expected {CALL_OPCODE:#04x}"
            ),
            Self::RelocationOffsetMismatch {
                call_index,
                expected,
                actual,
            } => write!(
                formatter,
                "call {call_index} relocation is at offset {actual}, expected {expected}"
            ),
            Self::ReturnOffsetMismatch {
                call_index,
                expected,
                actual,
            } => write!(
                formatter,
                "call {call_index} returns to offset {actual}, expected {expected}"
            ),
            Self::NonZeroRel32Placeholder {
                call_index,
                relocation_offset,
            } => write!(
                formatter,
                "call {call_index} has a nonzero rel32 placeholder at offset {relocation_offset}"
            ),
            Self::OverlappingCalls {
                call_index,
                previous_return_offset,
                call_offset,
            } => write!(
                formatter,
                "call {call_index} at offset {call_offset} overlaps the preceding call ending at {previous_return_offset}"
            ),
        }
    }
}

impl Error for CallRelocError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Encode(error) => Some(error),
            Self::Elf(error) => Some(error),
            _ => None,
        }
    }
}

impl From<EncodeError> for CallRelocError {
    fn from(error: EncodeError) -> Self {
        Self::Encode(error)
    }
}

impl From<ElfRelocError> for CallRelocError {
    fn from(error: ElfRelocError) -> Self {
        Self::Elf(error)
    }
}

pub fn emit_call_function(function: &CallFunction) -> Result<EncodedCallFunction, CallRelocError> {
    for (call_index, call) in function.calls.iter().enumerate() {
        if call.argument_count != 0 {
            return Err(CallRelocError::UnsupportedArguments {
                call_index,
                argument_count: call.argument_count,
            });
        }
        if let Some(reg) = call.live_caller_saved.first() {
            return Err(CallRelocError::CallerSavedLiveAcrossCall {
                call_index,
                reg: *reg,
            });
        }
    }

    let skeleton = emit_machine_code(&MachineFunction {
        name: function.name.clone(),
        instructions: vec![MachineInstr::Ret],
        stack_slots: 0,
        callee_saved: Vec::new(),
        symbols: Vec::new(),
    })?;
    let prologue_size = skeleton
        .len()
        .checked_sub(EPILOGUE_SIZE)
        .expect("x86_encode emits a return epilogue");
    let call_bytes = function
        .calls
        .len()
        .checked_mul(CALL_INSTRUCTION_SIZE as usize)
        .ok_or(CallRelocError::CodeSizeOverflow)?;
    let text_size = skeleton
        .len()
        .checked_add(call_bytes)
        .ok_or(CallRelocError::CodeSizeOverflow)?;
    let mut text = Vec::new();
    text.try_reserve_exact(text_size)
        .map_err(|_| CallRelocError::AllocationFailed)?;
    text.extend_from_slice(&skeleton[..prologue_size]);

    let mut call_sites = Vec::new();
    call_sites
        .try_reserve_exact(function.calls.len())
        .map_err(|_| CallRelocError::AllocationFailed)?;
    for call in &function.calls {
        let call_offset =
            u64::try_from(text.len()).map_err(|_| CallRelocError::CodeSizeOverflow)?;
        let relocation_offset = call_offset
            .checked_add(1)
            .ok_or(CallRelocError::CodeSizeOverflow)?;
        let return_offset = call_offset
            .checked_add(CALL_INSTRUCTION_SIZE)
            .ok_or(CallRelocError::CodeSizeOverflow)?;
        text.extend_from_slice(&[CALL_OPCODE, 0, 0, 0, 0]);
        call_sites.push(CallSite {
            call_offset,
            relocation_offset,
            return_offset,
            target: call.target.clone(),
        });
    }
    text.extend_from_slice(&skeleton[prologue_size..]);

    let encoded = EncodedCallFunction { text, call_sites };
    validate_call_sites(&encoded)?;
    Ok(encoded)
}

pub fn validate_call_sites(encoded: &EncodedCallFunction) -> Result<(), CallRelocError> {
    let text_size =
        u64::try_from(encoded.text.len()).map_err(|_| CallRelocError::CodeSizeOverflow)?;
    let mut previous_return_offset = 0;
    for (call_index, site) in encoded.call_sites.iter().enumerate() {
        let call_end = site.call_offset.checked_add(CALL_INSTRUCTION_SIZE).ok_or(
            CallRelocError::CallOffsetOutOfBounds {
                call_index,
                call_offset: site.call_offset,
                text_size,
            },
        )?;
        if call_end > text_size {
            return Err(CallRelocError::CallOffsetOutOfBounds {
                call_index,
                call_offset: site.call_offset,
                text_size,
            });
        }
        if call_index != 0 && site.call_offset < previous_return_offset {
            return Err(CallRelocError::OverlappingCalls {
                call_index,
                previous_return_offset,
                call_offset: site.call_offset,
            });
        }

        let call_offset = site.call_offset as usize;
        let actual = encoded.text[call_offset];
        if actual != CALL_OPCODE {
            return Err(CallRelocError::UnexpectedCallOpcode {
                call_index,
                call_offset: site.call_offset,
                actual,
            });
        }
        let expected_relocation = site.call_offset + 1;
        if site.relocation_offset != expected_relocation {
            return Err(CallRelocError::RelocationOffsetMismatch {
                call_index,
                expected: expected_relocation,
                actual: site.relocation_offset,
            });
        }
        if site.return_offset != call_end {
            return Err(CallRelocError::ReturnOffsetMismatch {
                call_index,
                expected: call_end,
                actual: site.return_offset,
            });
        }
        let relocation_offset = site.relocation_offset as usize;
        if encoded.text[relocation_offset..relocation_offset + 4]
            .iter()
            .any(|byte| *byte != 0)
        {
            return Err(CallRelocError::NonZeroRel32Placeholder {
                call_index,
                relocation_offset: site.relocation_offset,
            });
        }
        previous_return_offset = site.return_offset;
    }
    Ok(())
}

pub fn write_call_object(
    functions: &[CallFunction],
    external_symbols: &[String],
) -> Result<Vec<u8>, CallRelocError> {
    let mut encoded_functions = Vec::new();
    encoded_functions
        .try_reserve_exact(functions.len())
        .map_err(|_| CallRelocError::AllocationFailed)?;
    let mut text_size = 0_usize;
    let mut relocation_count = 0_usize;
    for function in functions {
        let encoded = emit_call_function(function)?;
        text_size = text_size
            .checked_add(encoded.text.len())
            .ok_or(CallRelocError::CodeSizeOverflow)?;
        relocation_count = relocation_count
            .checked_add(encoded.call_sites.len())
            .ok_or(CallRelocError::CodeSizeOverflow)?;
        encoded_functions.push(encoded);
    }

    let symbol_count = functions
        .len()
        .checked_add(external_symbols.len())
        .ok_or(CallRelocError::CodeSizeOverflow)?;
    let mut symbols = Vec::new();
    symbols
        .try_reserve_exact(symbol_count)
        .map_err(|_| CallRelocError::AllocationFailed)?;
    let mut function_offset = 0_u64;
    for (function, encoded) in functions.iter().zip(&encoded_functions) {
        let size =
            u64::try_from(encoded.text.len()).map_err(|_| CallRelocError::CodeSizeOverflow)?;
        symbols.push(ObjectSymbol::defined(
            function.name.clone(),
            function_offset,
            size,
        ));
        function_offset = function_offset
            .checked_add(size)
            .ok_or(CallRelocError::CodeSizeOverflow)?;
    }
    for name in external_symbols {
        symbols.push(ObjectSymbol::undefined(name.clone()));
    }

    let mut text = Vec::new();
    text.try_reserve_exact(text_size)
        .map_err(|_| CallRelocError::AllocationFailed)?;
    let mut relocations = Vec::new();
    relocations
        .try_reserve_exact(relocation_count)
        .map_err(|_| CallRelocError::AllocationFailed)?;
    for (function_index, encoded) in encoded_functions.iter().enumerate() {
        let base = u64::try_from(text.len()).map_err(|_| CallRelocError::CodeSizeOverflow)?;
        for (call_index, site) in encoded.call_sites.iter().enumerate() {
            let symbol_index = symbols
                .iter()
                .position(|symbol| symbol.name() == site.target)
                .ok_or_else(|| CallRelocError::UnknownTarget {
                    function_index,
                    call_index,
                    target: site.target.clone(),
                })?;
            let offset = base
                .checked_add(site.relocation_offset)
                .ok_or(CallRelocError::CodeSizeOverflow)?;
            relocations.push(Relocation::new(
                offset,
                symbol_index,
                R_X86_64_PLT32,
                CALL_RELOCATION_ADDEND,
            ));
        }
        text.extend_from_slice(&encoded.text);
    }
    Ok(write_object_with_relocations(
        &text,
        &symbols,
        &relocations,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::process::{self, Command};
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn emits_e8_placeholders_plt32_relocations_and_call_offsets() {
        let functions = [
            CallFunction::new("helper", Vec::new()),
            CallFunction::new(
                "caller",
                vec![DirectCall::new("helper"), DirectCall::new("external")],
            ),
        ];
        let helper = emit_call_function(&functions[0]).unwrap();
        let caller = emit_call_function(&functions[1]).unwrap();
        assert_eq!(helper.text, [0x55, 0x48, 0x89, 0xe5, 0xc9, 0xc3]);
        assert_eq!(
            caller.text,
            [
                0x55, 0x48, 0x89, 0xe5, 0xe8, 0, 0, 0, 0, 0xe8, 0, 0, 0, 0, 0xc9, 0xc3,
            ]
        );
        assert_eq!(
            caller.call_sites,
            [
                CallSite {
                    call_offset: 4,
                    relocation_offset: 5,
                    return_offset: 9,
                    target: "helper".into(),
                },
                CallSite {
                    call_offset: 9,
                    relocation_offset: 10,
                    return_offset: 14,
                    target: "external".into(),
                },
            ]
        );
        validate_call_sites(&caller).unwrap();

        let object = write_call_object(&functions, &["external".into()]).unwrap();
        assert_eq!(&object[64..86], &[helper.text, caller.text].concat());
        let rela = section_contents(&object, 2);
        assert_eq!(rela.len(), 48);
        assert_eq!(read_u64(rela, 0), 11);
        assert_eq!(read_u64(rela, 8), (1_u64 << 32) | u64::from(R_X86_64_PLT32));
        assert_eq!(read_i64(rela, 16), -4);
        assert_eq!(read_u64(rela, 24), 16);
        assert_eq!(
            read_u64(rela, 32),
            (3_u64 << 32) | u64::from(R_X86_64_PLT32)
        );
        assert_eq!(read_i64(rela, 40), -4);
    }

    #[test]
    fn rel32_addend_and_pushed_return_address_use_the_next_instruction() {
        let encoded = emit_call_function(&CallFunction::new(
            "caller",
            vec![DirectCall::new("helper"), DirectCall::new("external")],
        ))
        .unwrap();
        let first = &encoded.call_sites[0];
        let displacement =
            i64::from(CALL_RELOCATION_ADDEND as i32) - first.relocation_offset as i64;
        assert_eq!(displacement, -9);
        assert_eq!(first.return_offset as i64 + displacement, 0);

        let initial_rsp = 0x8000_u64;
        let mut stack = vec![0xdead_beef_u64];
        stack.push(first.return_offset);
        let callee_rsp = initial_rsp - 8;
        assert_eq!(stack.last(), Some(&first.return_offset));
        assert_eq!(
            first.return_offset,
            first.call_offset + CALL_INSTRUCTION_SIZE
        );
        assert_eq!(first.return_offset, encoded.call_sites[1].call_offset);
        let resumed_ip = stack.pop().unwrap();
        assert_eq!(resumed_ip, 9);
        assert_eq!(callee_rsp + 8, initial_rsp);
        assert_eq!(stack, [0xdead_beef]);

        let second = &encoded.call_sites[1];
        let external_address = 0x1000_i64;
        let displacement =
            external_address + CALL_RELOCATION_ADDEND - second.relocation_offset as i64;
        assert_eq!(displacement, 4082);
        assert_eq!(second.return_offset as i64 + displacement, external_address);
    }

    #[test]
    fn rejects_arguments_and_caller_saved_values_live_across_calls() {
        let with_arguments = CallFunction::new(
            "caller",
            vec![DirectCall {
                target: "helper".into(),
                argument_count: 1,
                live_caller_saved: Vec::new(),
            }],
        );
        assert_eq!(
            emit_call_function(&with_arguments),
            Err(CallRelocError::UnsupportedArguments {
                call_index: 0,
                argument_count: 1,
            })
        );

        for reg in Reg::CALLER_SAVED {
            let function = CallFunction::new(
                "caller",
                vec![DirectCall {
                    target: "helper".into(),
                    argument_count: 0,
                    live_caller_saved: vec![reg],
                }],
            );
            assert_eq!(
                emit_call_function(&function),
                Err(CallRelocError::CallerSavedLiveAcrossCall { call_index: 0, reg })
            );
        }
    }

    #[test]
    fn rejects_unknown_targets() {
        let functions = [CallFunction::new(
            "caller",
            vec![DirectCall::new("missing")],
        )];
        assert_eq!(
            write_call_object(&functions, &[]),
            Err(CallRelocError::UnknownTarget {
                function_index: 0,
                call_index: 0,
                target: "missing".into(),
            })
        );
    }

    #[test]
    fn validates_opcode_relocation_return_and_placeholder_offsets() {
        let valid = emit_call_function(&CallFunction::new(
            "caller",
            vec![DirectCall::new("helper")],
        ))
        .unwrap();

        let mut truncated = valid.clone();
        truncated.text.truncate(8);
        assert_eq!(
            validate_call_sites(&truncated),
            Err(CallRelocError::CallOffsetOutOfBounds {
                call_index: 0,
                call_offset: 4,
                text_size: 8,
            })
        );

        let mut wrong_opcode = valid.clone();
        wrong_opcode.text[4] = 0xe9;
        assert_eq!(
            validate_call_sites(&wrong_opcode),
            Err(CallRelocError::UnexpectedCallOpcode {
                call_index: 0,
                call_offset: 4,
                actual: 0xe9,
            })
        );

        let mut wrong_relocation = valid.clone();
        wrong_relocation.call_sites[0].relocation_offset = 4;
        assert_eq!(
            validate_call_sites(&wrong_relocation),
            Err(CallRelocError::RelocationOffsetMismatch {
                call_index: 0,
                expected: 5,
                actual: 4,
            })
        );

        let mut wrong_return = valid.clone();
        wrong_return.call_sites[0].return_offset = 8;
        assert_eq!(
            validate_call_sites(&wrong_return),
            Err(CallRelocError::ReturnOffsetMismatch {
                call_index: 0,
                expected: 9,
                actual: 8,
            })
        );

        let mut patched_placeholder = valid.clone();
        patched_placeholder.text[5] = 1;
        assert_eq!(
            validate_call_sites(&patched_placeholder),
            Err(CallRelocError::NonZeroRel32Placeholder {
                call_index: 0,
                relocation_offset: 5,
            })
        );
    }

    #[test]
    fn system_readelf_and_objdump_accept_calls_when_available() {
        let readelf = find_command(&["readelf", "llvm-readelf"]);
        let objdump = find_command(&["objdump", "llvm-objdump"]);
        if readelf.is_none() && objdump.is_none() {
            eprintln!("readelf/objdump not found; skipping external validation");
            return;
        }
        let object = write_call_object(
            &[
                CallFunction::new("helper", Vec::new()),
                CallFunction::new(
                    "caller",
                    vec![DirectCall::new("helper"), DirectCall::new("external")],
                ),
            ],
            &["external".into()],
        )
        .unwrap();
        let file = TempObject::new(&object);

        if let Some(readelf) = readelf {
            let output = Command::new(readelf)
                .args(["-W", "-r", "-s"])
                .arg(&file.0)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                output.status.success(),
                "{readelf} failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
            );
            for expected in [
                "R_X86_64_PLT32",
                "000000000000000b",
                "0000000000000010",
                "helper",
                "external",
                "UND",
            ] {
                assert!(
                    stdout.contains(expected),
                    "{readelf} output lacks {expected:?}:\n{stdout}"
                );
            }
        }

        if let Some(objdump) = objdump {
            let output = Command::new(objdump)
                .args(["-d", "-r", "-M", "intel"])
                .arg(&file.0)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                output.status.success(),
                "{objdump} failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
            );
            for expected in [
                "call",
                "R_X86_64_PLT32",
                "helper",
                "external",
                "leave",
                "ret",
            ] {
                assert!(
                    stdout.contains(expected),
                    "{objdump} output lacks {expected:?}:\n{stdout}"
                );
            }
        }
    }

    fn section_contents(object: &[u8], index: usize) -> &[u8] {
        let section_headers = read_u64(object, 40) as usize;
        let header = section_headers + index * 64;
        let offset = read_u64(object, header + 24) as usize;
        let size = read_u64(object, header + 32) as usize;
        &object[offset..offset + size]
    }

    fn read_u64(bytes: &[u8], offset: usize) -> u64 {
        u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
    }

    fn read_i64(bytes: &[u8], offset: usize) -> i64 {
        i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
    }

    fn find_command<'a>(commands: &'a [&'a str]) -> Option<&'a str> {
        commands.iter().copied().find(|command| {
            Command::new(command)
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success())
        })
    }

    struct TempObject(PathBuf);

    impl TempObject {
        fn new(contents: &[u8]) -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path =
                env::temp_dir().join(format!("kfrc-call-reloc-test-{}-{id}.o", process::id()));
            fs::write(&path, contents).unwrap();
            Self(path)
        }
    }

    impl Drop for TempObject {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
}
