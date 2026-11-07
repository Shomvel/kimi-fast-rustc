use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::elf_reloc::{
    ElfRelocError, ObjectSymbol, R_X86_64_PLT32, Relocation, write_object_with_relocations,
};
use crate::native::{self, CompileError};
use crate::son::Graph;
use crate::x86::MachineFunction;
use crate::x86_encode::{self, EncodeError};

#[derive(Debug)]
pub enum ProgramError {
    Compile(CompileError),
    Encode(EncodeError),
    Elf(ElfRelocError),
    UnsupportedParameters { function: String, count: usize },
    UnknownTarget { caller: String, target: String },
}

impl fmt::Display for ProgramError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compile(error) => error.fmt(formatter),
            Self::Encode(error) => error.fmt(formatter),
            Self::Elf(error) => error.fmt(formatter),
            Self::UnsupportedParameters { function, count } => write!(
                formatter,
                "function {function} has {count} parameters; only zero-argument functions are supported"
            ),
            Self::UnknownTarget { caller, target } => {
                write!(
                    formatter,
                    "function {caller} calls unknown function {target}"
                )
            }
        }
    }
}

impl Error for ProgramError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Compile(error) => Some(error),
            Self::Encode(error) => Some(error),
            Self::Elf(error) => Some(error),
            _ => None,
        }
    }
}

impl From<CompileError> for ProgramError {
    fn from(error: CompileError) -> Self {
        Self::Compile(error)
    }
}

impl From<EncodeError> for ProgramError {
    fn from(error: EncodeError) -> Self {
        Self::Encode(error)
    }
}

impl From<ElfRelocError> for ProgramError {
    fn from(error: ElfRelocError) -> Self {
        Self::Elf(error)
    }
}

#[derive(Clone, Debug)]
pub struct NativeProgram {
    pub functions: Vec<MachineFunction>,
}

impl NativeProgram {
    pub fn emit_gnu_assembly(&self) -> Result<String, CompileError> {
        let mut assembly = String::new();
        for function in &self.functions {
            assembly.push_str(&function.emit_gnu_assembly()?);
        }
        Ok(assembly)
    }

    pub fn emit_object(&self) -> Result<Vec<u8>, ProgramError> {
        let mut text = Vec::new();
        let mut symbols = Vec::new();
        let mut relocations = Vec::new();
        let mut call_sites = Vec::new();
        let mut symbol_indices = BTreeMap::new();

        for function in &self.functions {
            let index = symbols.len();
            symbol_indices.insert(function.name.clone(), index);
            symbols.push(ObjectSymbol::defined(
                function.name.clone(),
                text.len() as u64,
                0,
            ));
            let (code, calls) = x86_encode::encode_with_relocations(function)?;
            let text_offset = text.len() as u64;
            let code_size = code.len() as u64;
            text.extend_from_slice(&code);

            let ObjectSymbol::Defined(symbol) = &mut symbols[index] else {
                unreachable!()
            };
            symbol.size = code_size;
            for call in calls {
                call_sites.push((
                    text_offset + call.offset,
                    call.target,
                    function.name.clone(),
                ));
            }
        }

        for (offset, target, caller) in call_sites {
            let symbol_index = match symbol_indices.get(&target).copied() {
                Some(index) => index,
                None => {
                    let index = symbols.len();
                    symbol_indices.insert(target.clone(), index);
                    symbols.push(ObjectSymbol::undefined(target.clone()));
                    index
                }
            };
            if !symbol_indices.contains_key(&caller) {
                return Err(ProgramError::UnknownTarget { caller, target });
            }
            relocations.push(Relocation::new(offset, symbol_index, R_X86_64_PLT32, -4));
        }
        Ok(write_object_with_relocations(
            &text,
            &symbols,
            &relocations,
        )?)
    }
}

pub fn compile_graph(graph: &Graph) -> Result<NativeProgram, ProgramError> {
    if graph.functions.is_empty() {
        return Ok(NativeProgram {
            functions: vec![native::compile_graph(graph, "main")?],
        });
    }
    let mut functions = Vec::new();
    for metadata in &graph.functions {
        functions.push(native::compile_graph(
            &graph.function_graph(metadata),
            metadata.name.clone(),
        )?);
    }
    Ok(NativeProgram { functions })
}

pub fn compile_graph_to_assembly(graph: &Graph) -> Result<String, ProgramError> {
    Ok(compile_graph(graph)?.emit_gnu_assembly()?)
}

pub fn compile_graph_to_object(graph: &Graph) -> Result<Vec<u8>, ProgramError> {
    compile_graph(graph)?.emit_object()
}
