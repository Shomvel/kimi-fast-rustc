//! ELF relocatable objects from encoded functions.

use std::collections::{HashMap, HashSet};

use object::write::{Object, Relocation, StandardSection, Symbol, SymbolSection};
use object::{
    Architecture, BinaryFormat, Endianness, RelocationEncoding, RelocationFlags, RelocationKind,
    SymbolFlags, SymbolKind, SymbolScope,
};

use kfrc_body::{DataBinding, DataObject};
use crate::machine::x86::MachineFunction;
use crate::machine::x86_encode::{
    AddressRelocationKind, EncodeError, encode_with_all_relocations,
};

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
    SymbolKindMismatch { symbol: String, thread_local: bool },
    DuplicateSymbol(String),
}

impl std::fmt::Display for ObjectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateSymbol(symbol) => write!(
                formatter,
                "duplicate definition of `{symbol}` in one object"
            ),
            Self::Encode { symbol, error } => {
                write!(formatter, "encoding `{symbol}` failed: {error:?}")
            }
            Self::SymbolKindMismatch {
                symbol,
                thread_local,
            } => write!(
                formatter,
                "symbol `{symbol}` is used as {} but has a conflicting storage kind",
                if *thread_local {
                    "TLS"
                } else {
                    "ordinary storage"
                }
            ),
            Self::Write(error) => write!(formatter, "writing the object failed: {error}"),
        }
    }
}

impl std::error::Error for ObjectError {}

/// Writes all functions into one `.text` section, in the given order, with
/// `R_X86_64_PLT32` relocations for calls.
pub fn write_object(functions: &[FunctionCode]) -> Result<Vec<u8>, ObjectError> {
    write_object_with_data(functions, &[])
}

pub fn write_object_with_data(
    functions: &[FunctionCode],
    data: &[DataObject],
) -> Result<Vec<u8>, ObjectError> {
    let mut object = Object::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    object.add_file_symbol(b"kfrc".to_vec());
    let text = object.section_id(StandardSection::Text);
    object.add_section(
        Vec::new(),
        b".note.GNU-stack".to_vec(),
        object::SectionKind::Linker,
    );
    let mut pending_relocations = Vec::new();
    let mut storage_kinds = HashMap::new();
    let mut definitions = HashSet::new();
    for datum in data {
        if !definitions.insert(datum.symbol.as_str()) {
            return Err(ObjectError::DuplicateSymbol(datum.symbol.clone()));
        }
        require_storage_kind(&mut storage_kinds, &datum.symbol, datum.thread_local)?;
    }
    for function in functions {
        if !definitions.insert(function.function.name.as_str()) {
            return Err(ObjectError::DuplicateSymbol(function.function.name.clone()));
        }
        require_storage_kind(&mut storage_kinds, &function.function.name, false)?;
    }
    for code in functions {
        let symbol_name = code.function.name.clone();
        let (bytes, calls, addresses) =
            encode_with_all_relocations(&code.function).map_err(|error| ObjectError::Encode {
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
            pending_relocations.push((
                text,
                offset + call.offset,
                call.target,
                -4,
                RelocationFlags::Generic {
                    kind: RelocationKind::PltRelative,
                    encoding: RelocationEncoding::X86Branch,
                    size: 32,
                },
                false,
            ));
        }
        for address in addresses {
            pending_relocations.push((
                text,
                offset + address.offset,
                address.target,
                match address.kind {
                    AddressRelocationKind::PcRelative => address.addend - 4,
                    AddressRelocationKind::TlsLocalExec => address.addend,
                },
                match address.kind {
                    AddressRelocationKind::PcRelative => RelocationFlags::Generic {
                        kind: RelocationKind::Relative,
                        encoding: RelocationEncoding::Generic,
                        size: 32,
                    },
                    AddressRelocationKind::TlsLocalExec => RelocationFlags::Elf {
                        r_type: object::elf::R_X86_64_TPOFF32,
                    },
                },
                address.kind == AddressRelocationKind::TlsLocalExec,
            ));
        }
    }
    let readonly = object.section_id(StandardSection::ReadOnlyData);
    for datum in data {
        let zero_tls = datum.thread_local
            && datum.relocations.is_empty()
            && datum.bytes.iter().all(|byte| *byte == 0);
        let section = if zero_tls {
            object.section_id(StandardSection::UninitializedTls)
        } else if datum.thread_local {
            object.section_id(StandardSection::Tls)
        } else if datum.writable {
            object.section_id(StandardSection::Data)
        } else if datum.relocations.is_empty() {
            readonly
        } else {
            object.section_id(StandardSection::ReadOnlyDataWithRel)
        };
        let offset = if zero_tls {
            object.append_section_bss(section, datum.bytes.len() as u64, datum.align.max(1))
        } else {
            object.append_section_data(section, &datum.bytes, datum.align.max(1))
        };
        object.add_symbol(Symbol {
            name: datum.symbol.as_bytes().to_vec(),
            value: offset,
            size: datum.bytes.len() as u64,
            kind: if datum.thread_local {
                SymbolKind::Tls
            } else {
                SymbolKind::Data
            },
            scope: match datum.binding {
                DataBinding::Local => SymbolScope::Compilation,
                DataBinding::Global { hidden: true } => SymbolScope::Linkage,
                DataBinding::Global { hidden: false } => SymbolScope::Dynamic,
            },
            weak: false,
            section: SymbolSection::Section(section),
            flags: SymbolFlags::None,
        });
        for relocation in &datum.relocations {
            pending_relocations.push((
                section,
                offset + relocation.offset,
                relocation.target.clone(),
                relocation.addend,
                RelocationFlags::Generic {
                    kind: RelocationKind::Absolute,
                    encoding: RelocationEncoding::Generic,
                    size: 64,
                },
                false,
            ));
        }
    }
    for (section, offset, target, addend, flags, thread_local) in pending_relocations {
        require_storage_kind(&mut storage_kinds, &target, thread_local)?;
        let symbol = match object.symbol_id(target.as_bytes()) {
            Some(symbol) => symbol,
            None => object.add_symbol(Symbol {
                name: target.into_bytes(),
                value: 0,
                size: 0,
                kind: if thread_local {
                    SymbolKind::Tls
                } else {
                    SymbolKind::Unknown
                },
                scope: SymbolScope::Dynamic,
                weak: false,
                section: SymbolSection::Undefined,
                flags: SymbolFlags::None,
            }),
        };
        object
            .add_relocation(
                section,
                Relocation {
                    offset,
                    symbol,
                    addend,
                    flags,
                },
            )
            .map_err(ObjectError::Write)?;
    }
    object.write().map_err(ObjectError::Write)
}

fn require_storage_kind(
    kinds: &mut HashMap<String, bool>,
    symbol: &str,
    thread_local: bool,
) -> Result<(), ObjectError> {
    match kinds.get(symbol) {
        Some(existing) if *existing != thread_local => Err(ObjectError::SymbolKindMismatch {
            symbol: symbol.to_owned(),
            thread_local,
        }),
        Some(_) => Ok(()),
        None => {
            kinds.insert(symbol.to_owned(), thread_local);
            Ok(())
        }
    }
}
