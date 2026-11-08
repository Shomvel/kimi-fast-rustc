//! ELF relocatable objects from encoded functions.

use object::write::{Object, Relocation, StandardSection, Symbol, SymbolSection};
use object::{
    Architecture, BinaryFormat, Endianness, RelocationEncoding, RelocationFlags, RelocationKind,
    SymbolFlags, SymbolKind, SymbolScope,
};

use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::x86_encode::{EncodeError, encode_with_relocations};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolBinding {
    /// Visible to other objects (`STB_GLOBAL`).
    Global { hidden: bool },
    /// Only within this object (`STB_LOCAL`).
    Local,
}

pub struct FunctionCode {
    pub function: MachineFunction,
    pub binding: SymbolBinding,
}

#[derive(Debug)]
pub enum ObjectError {
    Encode { symbol: String, error: EncodeError },
    Write(object::write::Error),
}

impl std::fmt::Display for ObjectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Encode { symbol, error } => write!(formatter, "encoding `{symbol}` failed: {error:?}"),
            Self::Write(error) => write!(formatter, "writing the object failed: {error}"),
        }
    }
}

impl std::error::Error for ObjectError {}

/// Writes all functions into one `.text` section, in the given order, with
/// `R_X86_64_PLT32` relocations for calls.
pub fn write_object(functions: &[FunctionCode]) -> Result<Vec<u8>, ObjectError> {
    let mut object = Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    object.add_file_symbol(b"kfrc".to_vec());
    let text = object.section_id(StandardSection::Text);
    let mut pending_relocations = Vec::new();
    for code in functions {
        let symbol_name = code.function.name.clone();
        let (bytes, calls) = encode_with_relocations(&code.function).map_err(|error| ObjectError::Encode {
            symbol: symbol_name.clone(),
            error,
        })?;
        let offset = object.append_section_data(text, &bytes, 16);
        let (scope, weak) = match code.binding {
            SymbolBinding::Global { hidden: false } => (SymbolScope::Dynamic, false),
            SymbolBinding::Global { hidden: true } => (SymbolScope::Linkage, false),
            SymbolBinding::Local => (SymbolScope::Compilation, false),
        };
        object.add_symbol(Symbol {
            name: symbol_name.into_bytes(),
            value: offset,
            size: bytes.len() as u64,
            kind: SymbolKind::Text,
            scope,
            weak,
            section: SymbolSection::Section(text),
            flags: SymbolFlags::None,
        });
        for call in calls {
            pending_relocations.push((offset + call.offset, call.target));
        }
    }
    for (offset, target) in pending_relocations {
        let symbol = match object.symbol_id(target.as_bytes()) {
            Some(symbol) => symbol,
            None => object.add_symbol(Symbol {
                name: target.into_bytes(),
                value: 0,
                size: 0,
                kind: SymbolKind::Text,
                scope: SymbolScope::Dynamic,
                weak: false,
                section: SymbolSection::Undefined,
                flags: SymbolFlags::None,
            }),
        };
        object
            .add_relocation(
                text,
                Relocation {
                    offset,
                    symbol,
                    addend: -4,
                    flags: RelocationFlags::Generic {
                        kind: RelocationKind::PltRelative,
                        encoding: RelocationEncoding::X86Branch,
                        size: 32,
                    },
                },
            )
            .map_err(ObjectError::Write)?;
    }
    object.write().map_err(ObjectError::Write)
}
