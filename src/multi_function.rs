//! Self-contained ELF64 objects containing native functions and direct callers.
//!
//! The supported ABI is deliberately narrow: Linux x86_64 System V functions
//! with no parameters and one i64 result in `rax`. Native functions that read an
//! argument are rejected, as are calls with arguments or values that must stay
//! live in caller-saved registers. Variadic functions, floating-point and
//! aggregate values, stack parameters, other architectures, and other ABIs are
//! unsupported. A caller must return immediately after its calls so the last
//! callee's `rax` value remains the caller's result.

use crate::call_reloc::{
    CALL_RELOCATION_ADDEND, CallFunction, CallRelocError, DirectCall, EncodedCallFunction,
    emit_call_function,
};
use crate::elf_reloc::{
    ElfRelocError, ObjectSymbol, R_X86_64_PLT32, Relocation, write_object_with_relocations,
};
use crate::native::{CompileError, compile_graph};
use crate::son::Graph;
use crate::x86::{MachineFunction, MachineInstr};
use crate::x86_encode::EncodeError;
use std::error::Error;
use std::fmt;

pub const NATIVE_ANSWER: i64 = 42;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MultiFunctionError {
    Compile(CompileError),
    Encode(EncodeError),
    Call(CallRelocError),
    Elf(ElfRelocError),
    UnsupportedNativeArguments {
        function_index: usize,
        function_name: String,
        argument_index: usize,
    },
    CodeSizeOverflow,
    AllocationFailed,
}

impl fmt::Display for MultiFunctionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compile(error) => write!(formatter, "native graph compilation failed: {error}"),
            Self::Encode(error) => write!(formatter, "x86_64 encoding failed: {error}"),
            Self::Call(error) => write!(formatter, "direct-call emission failed: {error}"),
            Self::Elf(error) => write!(formatter, "ELF object emission failed: {error}"),
            Self::UnsupportedNativeArguments {
                function_index,
                function_name,
                argument_index,
            } => write!(
                formatter,
                "native function {function_index} ({function_name:?}) reads argument {argument_index}; only zero-parameter i64 System V functions are supported"
            ),
            Self::CodeSizeOverflow => formatter.write_str("multi-function object size overflows"),
            Self::AllocationFailed => {
                formatter.write_str("could not allocate the multi-function object")
            }
        }
    }
}

impl Error for MultiFunctionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Compile(error) => Some(error),
            Self::Encode(error) => Some(error),
            Self::Call(error) => Some(error),
            Self::Elf(error) => Some(error),
            _ => None,
        }
    }
}

impl From<CompileError> for MultiFunctionError {
    fn from(error: CompileError) -> Self {
        Self::Compile(error)
    }
}

impl From<EncodeError> for MultiFunctionError {
    fn from(error: EncodeError) -> Self {
        Self::Encode(error)
    }
}

impl From<CallRelocError> for MultiFunctionError {
    fn from(error: CallRelocError) -> Self {
        Self::Call(error)
    }
}

impl From<ElfRelocError> for MultiFunctionError {
    fn from(error: ElfRelocError) -> Self {
        Self::Elf(error)
    }
}

/// Emits a self-contained object whose `main` calls `native_answer` and returns
/// its i64 result. `main` intentionally ignores any incoming process arguments.
pub fn write_native_answer_object() -> Result<Vec<u8>, MultiFunctionError> {
    let mut graph = Graph::new();
    let start = graph.start();
    let answer = graph.constant(NATIVE_ANSWER);
    graph.return_(start, Some(answer));
    let native_answer = compile_graph(&graph, "native_answer")?;
    let main = CallFunction::new("main", vec![DirectCall::new("native_answer")]);
    write_multi_function_object(&[native_answer], &[main])
}

/// Combines zero-parameter native functions and direct-call functions in one
/// `.text` section. Native functions precede callers; every call target must be
/// defined in one of the two slices. No undefined symbols are emitted.
pub fn write_multi_function_object(
    native_functions: &[MachineFunction],
    call_functions: &[CallFunction],
) -> Result<Vec<u8>, MultiFunctionError> {
    let mut native_code = Vec::new();
    native_code
        .try_reserve_exact(native_functions.len())
        .map_err(|_| MultiFunctionError::AllocationFailed)?;
    let mut text_size = 0_usize;
    for (function_index, function) in native_functions.iter().enumerate() {
        if let Some(argument_index) = first_argument_index(function) {
            return Err(MultiFunctionError::UnsupportedNativeArguments {
                function_index,
                function_name: function.name.clone(),
                argument_index,
            });
        }
        let code = function.emit_machine_code()?;
        text_size = text_size
            .checked_add(code.len())
            .ok_or(MultiFunctionError::CodeSizeOverflow)?;
        native_code.push(code);
    }

    let mut call_code = Vec::new();
    call_code
        .try_reserve_exact(call_functions.len())
        .map_err(|_| MultiFunctionError::AllocationFailed)?;
    let mut relocation_count = 0_usize;
    for function in call_functions {
        let encoded = emit_call_function(function)?;
        text_size = text_size
            .checked_add(encoded.text.len())
            .ok_or(MultiFunctionError::CodeSizeOverflow)?;
        relocation_count = relocation_count
            .checked_add(encoded.call_sites.len())
            .ok_or(MultiFunctionError::CodeSizeOverflow)?;
        call_code.push(encoded);
    }

    let symbols = object_symbols(native_functions, &native_code, call_functions, &call_code)?;
    let (text, relocations) = combine_text_and_relocations(
        &native_code,
        &call_code,
        &symbols,
        text_size,
        relocation_count,
    )?;
    Ok(write_object_with_relocations(
        &text,
        &symbols,
        &relocations,
    )?)
}

fn first_argument_index(function: &MachineFunction) -> Option<usize> {
    function.instructions.iter().find_map(|instruction| {
        if let MachineInstr::LoadArg { index, .. } = *instruction {
            Some(index)
        } else {
            None
        }
    })
}

fn object_symbols(
    native_functions: &[MachineFunction],
    native_code: &[Vec<u8>],
    call_functions: &[CallFunction],
    call_code: &[EncodedCallFunction],
) -> Result<Vec<ObjectSymbol>, MultiFunctionError> {
    let symbol_count = native_functions
        .len()
        .checked_add(call_functions.len())
        .ok_or(MultiFunctionError::CodeSizeOverflow)?;
    let mut symbols = Vec::new();
    symbols
        .try_reserve_exact(symbol_count)
        .map_err(|_| MultiFunctionError::AllocationFailed)?;
    let mut offset = 0_u64;
    for (function, code) in native_functions.iter().zip(native_code) {
        push_defined_symbol(&mut symbols, function.name.clone(), &mut offset, code.len())?;
    }
    for (function, code) in call_functions.iter().zip(call_code) {
        push_defined_symbol(
            &mut symbols,
            function.name.clone(),
            &mut offset,
            code.text.len(),
        )?;
    }
    Ok(symbols)
}

fn push_defined_symbol(
    symbols: &mut Vec<ObjectSymbol>,
    name: String,
    offset: &mut u64,
    size: usize,
) -> Result<(), MultiFunctionError> {
    let size = u64::try_from(size).map_err(|_| MultiFunctionError::CodeSizeOverflow)?;
    symbols.push(ObjectSymbol::defined(name, *offset, size));
    *offset = offset
        .checked_add(size)
        .ok_or(MultiFunctionError::CodeSizeOverflow)?;
    Ok(())
}

fn combine_text_and_relocations(
    native_code: &[Vec<u8>],
    call_code: &[EncodedCallFunction],
    symbols: &[ObjectSymbol],
    text_size: usize,
    relocation_count: usize,
) -> Result<(Vec<u8>, Vec<Relocation>), MultiFunctionError> {
    let mut text = Vec::new();
    text.try_reserve_exact(text_size)
        .map_err(|_| MultiFunctionError::AllocationFailed)?;
    let mut relocations = Vec::new();
    relocations
        .try_reserve_exact(relocation_count)
        .map_err(|_| MultiFunctionError::AllocationFailed)?;

    for code in native_code {
        text.extend_from_slice(code);
    }
    for (function_index, encoded) in call_code.iter().enumerate() {
        let base = u64::try_from(text.len()).map_err(|_| MultiFunctionError::CodeSizeOverflow)?;
        for (call_index, site) in encoded.call_sites.iter().enumerate() {
            let symbol_index = symbols
                .iter()
                .position(|symbol| symbol.name() == site.target)
                .ok_or_else(|| {
                    MultiFunctionError::Call(CallRelocError::UnknownTarget {
                        function_index,
                        call_index,
                        target: site.target.clone(),
                    })
                })?;
            let offset = base
                .checked_add(site.relocation_offset)
                .ok_or(MultiFunctionError::CodeSizeOverflow)?;
            relocations.push(Relocation::new(
                offset,
                symbol_index,
                R_X86_64_PLT32,
                CALL_RELOCATION_ADDEND,
            ));
        }
        text.extend_from_slice(&encoded.text);
    }
    Ok((text, relocations))
}
