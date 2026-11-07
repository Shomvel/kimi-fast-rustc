//! Minimal ELF64 x86_64 relocatable object emission with `.rela.text`.
//!
//! This writer handles only static `R_X86_64_PC32` and `R_X86_64_PLT32`
//! relocations against defined or undefined global symbols. It does not emit
//! thread-local storage, dynamic linking sections, or other relocation kinds.

use crate::elf::{ElfError, FunctionSymbol};
use std::collections::HashSet;
use std::error::Error;
use std::fmt;

pub const R_X86_64_PC32: u32 = 2;
pub const R_X86_64_PLT32: u32 = 4;

const ELF_HEADER_SIZE: u64 = 64;
const SECTION_HEADER_SIZE: u64 = 64;
const SYMBOL_SIZE: u64 = 24;
const RELA_SIZE: u64 = 24;
const RELOCATION_WIDTH: u64 = 4;
const SECTION_COUNT: u64 = 6;
const TEXT_SECTION_INDEX: u16 = 1;
#[cfg(test)]
const RELA_TEXT_SECTION_INDEX: u16 = 2;
const SYMTAB_SECTION_INDEX: u32 = 3;
const STRTAB_SECTION_INDEX: u32 = 4;
const SHSTRTAB_SECTION_INDEX: u16 = 5;
const SHSTRTAB: &[u8] = b"\0.text\0.rela.text\0.symtab\0.strtab\0.shstrtab\0";
const TEXT_NAME_OFFSET: u32 = 1;
const RELA_TEXT_NAME_OFFSET: u32 = 7;
const SYMTAB_NAME_OFFSET: u32 = 18;
const STRTAB_NAME_OFFSET: u32 = 26;
const SHSTRTAB_NAME_OFFSET: u32 = 34;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObjectSymbol {
    Defined(FunctionSymbol),
    Undefined(String),
}

impl ObjectSymbol {
    pub fn defined(name: impl Into<String>, offset: u64, size: u64) -> Self {
        Self::Defined(FunctionSymbol::new(name, offset, size))
    }

    pub fn undefined(name: impl Into<String>) -> Self {
        Self::Undefined(name.into())
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Defined(symbol) => &symbol.name,
            Self::Undefined(name) => name,
        }
    }
}

impl From<FunctionSymbol> for ObjectSymbol {
    fn from(symbol: FunctionSymbol) -> Self {
        Self::Defined(symbol)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Relocation {
    pub offset: u64,
    pub symbol_index: usize,
    pub relocation_type: u32,
    pub addend: i64,
}

impl Relocation {
    pub fn new(offset: u64, symbol_index: usize, relocation_type: u32, addend: i64) -> Self {
        Self {
            offset,
            symbol_index,
            relocation_type,
            addend,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ElfRelocError {
    Symbol(ElfError),
    DuplicateSymbolName {
        index: usize,
    },
    UnsupportedRelocationType {
        index: usize,
        relocation_type: u32,
    },
    RelocationSymbolOutOfBounds {
        index: usize,
        symbol_index: usize,
        symbol_count: usize,
    },
    RelocationOffsetOutOfBounds {
        index: usize,
        offset: u64,
        text_size: u64,
    },
    RelocationAddendOutOfRange {
        index: usize,
        addend: i64,
    },
}

impl fmt::Display for ElfRelocError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Symbol(error) => error.fmt(formatter),
            Self::DuplicateSymbolName { index } => {
                write!(formatter, "object symbol {index} has a duplicate name")
            }
            Self::UnsupportedRelocationType {
                index,
                relocation_type,
            } => write!(
                formatter,
                "relocation {index} has unsupported x86_64 type {relocation_type}; only R_X86_64_PC32 and R_X86_64_PLT32 are supported"
            ),
            Self::RelocationSymbolOutOfBounds {
                index,
                symbol_index,
                symbol_count,
            } => write!(
                formatter,
                "relocation {index} references symbol {symbol_index}, but the object has {symbol_count} symbols"
            ),
            Self::RelocationOffsetOutOfBounds {
                index,
                offset,
                text_size,
            } => write!(
                formatter,
                "relocation {index} at offset {offset} needs {RELOCATION_WIDTH} bytes in .text of size {text_size}"
            ),
            Self::RelocationAddendOutOfRange { index, addend } => write!(
                formatter,
                "relocation {index} addend {addend} does not fit a signed 32-bit PC-relative field"
            ),
        }
    }
}

impl Error for ElfRelocError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Symbol(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ElfError> for ElfRelocError {
    fn from(error: ElfError) -> Self {
        Self::Symbol(error)
    }
}

#[derive(Debug)]
struct Layout {
    text_size: u64,
    rela_offset: u64,
    rela_size: u64,
    symtab_offset: u64,
    symtab_size: u64,
    strtab_offset: u64,
    strtab_size: u64,
    shstrtab_offset: u64,
    section_headers_offset: u64,
    file_size: u64,
}

impl Layout {
    fn new(
        text_size: u64,
        relocation_count: u64,
        symbol_count: u64,
        strtab_size: u64,
    ) -> Result<Self, ElfError> {
        let text_end = checked_add(ELF_HEADER_SIZE, text_size, "relocation section offset")?;
        let rela_offset = align_offset(text_end, 8, "relocation section offset")?;
        let rela_size = relocation_count
            .checked_mul(RELA_SIZE)
            .ok_or(ElfError::FieldOverflow {
                field: "relocation section size",
            })?;
        let symtab_offset = checked_add(rela_offset, rela_size, "symbol table offset")?;
        let symtab_size = symbol_count
            .checked_add(1)
            .and_then(|count| count.checked_mul(SYMBOL_SIZE))
            .ok_or(ElfError::FieldOverflow {
                field: "symbol table size",
            })?;
        let strtab_offset = checked_add(symtab_offset, symtab_size, "string table offset")?;
        let shstrtab_offset = checked_add(
            strtab_offset,
            strtab_size,
            "section-name string table offset",
        )?;
        let shstrtab_size = u64::try_from(SHSTRTAB.len()).map_err(|_| ElfError::FieldOverflow {
            field: "section-name string table size",
        })?;
        let section_headers_end =
            checked_add(shstrtab_offset, shstrtab_size, "section header offset")?;
        let section_headers_offset = align_offset(section_headers_end, 8, "section header offset")?;
        let section_headers_size =
            SECTION_COUNT
                .checked_mul(SECTION_HEADER_SIZE)
                .ok_or(ElfError::FieldOverflow {
                    field: "section header table size",
                })?;
        let file_size = checked_add(section_headers_offset, section_headers_size, "object size")?;

        Ok(Self {
            text_size,
            rela_offset,
            rela_size,
            symtab_offset,
            symtab_size,
            strtab_offset,
            strtab_size,
            shstrtab_offset,
            section_headers_offset,
            file_size,
        })
    }
}

#[derive(Default)]
struct SectionHeader {
    name: u32,
    kind: u32,
    flags: u64,
    offset: u64,
    size: u64,
    link: u32,
    info: u32,
    alignment: u64,
    entry_size: u64,
}

/// Writes an ELF64 x86_64 relocatable object with `.text`, `.rela.text`, and a
/// global symbol table. Relocation symbol indices refer to `symbols`; index zero
/// in the ELF symbol table remains the mandatory undefined null symbol.
pub fn write_object_with_relocations(
    text: &[u8],
    symbols: &[ObjectSymbol],
    relocations: &[Relocation],
) -> Result<Vec<u8>, ElfRelocError> {
    let text_size =
        u64::try_from(text.len()).map_err(|_| ElfError::FieldOverflow { field: "text size" })?;
    let relocation_count =
        u64::try_from(relocations.len()).map_err(|_| ElfError::FieldOverflow {
            field: "relocation count",
        })?;
    let symbol_count = u64::try_from(symbols.len()).map_err(|_| ElfError::FieldOverflow {
        field: "symbol count",
    })?;
    if symbol_count > u64::from(u32::MAX) {
        return Err(ElfError::FieldOverflow {
            field: "symbol table index",
        }
        .into());
    }

    let mut names = HashSet::new();
    names
        .try_reserve(symbols.len())
        .map_err(|_| ElfRelocError::from(ElfError::AllocationFailed))?;
    let mut name_offsets = Vec::new();
    name_offsets
        .try_reserve_exact(symbols.len())
        .map_err(|_| ElfRelocError::from(ElfError::AllocationFailed))?;
    let mut strtab_size = 1_u64;

    for (index, symbol) in symbols.iter().enumerate() {
        if symbol.name().is_empty() {
            return Err(ElfError::EmptySymbolName { index }.into());
        }
        if symbol.name().as_bytes().contains(&0) {
            return Err(ElfError::SymbolNameContainsNul { index }.into());
        }
        if !names.insert(symbol.name()) {
            return Err(ElfRelocError::DuplicateSymbolName { index });
        }
        if let ObjectSymbol::Defined(symbol) = symbol {
            if symbol.offset > text_size || symbol.size > text_size - symbol.offset {
                return Err(ElfError::SymbolOutOfBounds {
                    index,
                    offset: symbol.offset,
                    size: symbol.size,
                    text_size,
                }
                .into());
            }
        }

        name_offsets.push(symbol_name_offset(strtab_size)?);
        let name_size =
            u64::try_from(symbol.name().len()).map_err(|_| ElfError::FieldOverflow {
                field: "string table size",
            })?;
        strtab_size = checked_add(strtab_size, name_size, "string table size")?;
        strtab_size = checked_add(strtab_size, 1, "string table size")?;
    }

    for (index, relocation) in relocations.iter().enumerate() {
        if relocation.relocation_type != R_X86_64_PC32
            && relocation.relocation_type != R_X86_64_PLT32
        {
            return Err(ElfRelocError::UnsupportedRelocationType {
                index,
                relocation_type: relocation.relocation_type,
            });
        }
        if relocation.symbol_index >= symbols.len() {
            return Err(ElfRelocError::RelocationSymbolOutOfBounds {
                index,
                symbol_index: relocation.symbol_index,
                symbol_count: symbols.len(),
            });
        }
        if relocation.offset > text_size || text_size - relocation.offset < RELOCATION_WIDTH {
            return Err(ElfRelocError::RelocationOffsetOutOfBounds {
                index,
                offset: relocation.offset,
                text_size,
            });
        }
        if i32::try_from(relocation.addend).is_err() {
            return Err(ElfRelocError::RelocationAddendOutOfRange {
                index,
                addend: relocation.addend,
            });
        }
    }

    let layout = Layout::new(text_size, relocation_count, symbol_count, strtab_size)?;
    let output_size = usize::try_from(layout.file_size).map_err(|_| {
        ElfRelocError::from(ElfError::FieldOverflow {
            field: "object size",
        })
    })?;
    let mut object = Vec::new();
    object
        .try_reserve_exact(output_size)
        .map_err(|_| ElfRelocError::from(ElfError::AllocationFailed))?;

    write_elf_header(&mut object, layout.section_headers_offset);
    object.extend_from_slice(text);
    pad_to(&mut object, layout.rela_offset);

    for relocation in relocations {
        let elf_symbol_index =
            u64::try_from(relocation.symbol_index).expect("a validated symbol index fits u64") + 1;
        push_u64(&mut object, relocation.offset);
        push_u64(
            &mut object,
            (elf_symbol_index << 32) | u64::from(relocation.relocation_type),
        );
        push_i64(&mut object, relocation.addend);
    }
    debug_assert_eq!(object.len() as u64, layout.symtab_offset);

    object.extend_from_slice(&[0; SYMBOL_SIZE as usize]);
    for (symbol, name_offset) in symbols.iter().zip(name_offsets) {
        write_object_symbol(&mut object, name_offset, symbol);
    }
    debug_assert_eq!(object.len() as u64, layout.strtab_offset);

    object.push(0);
    for symbol in symbols {
        object.extend_from_slice(symbol.name().as_bytes());
        object.push(0);
    }
    debug_assert_eq!(object.len() as u64, layout.shstrtab_offset);

    object.extend_from_slice(SHSTRTAB);
    pad_to(&mut object, layout.section_headers_offset);

    write_section_header(&mut object, &SectionHeader::default());
    write_section_header(
        &mut object,
        &SectionHeader {
            name: TEXT_NAME_OFFSET,
            kind: 1,
            flags: 0x6,
            offset: ELF_HEADER_SIZE,
            size: layout.text_size,
            alignment: 16,
            ..SectionHeader::default()
        },
    );
    write_section_header(
        &mut object,
        &SectionHeader {
            name: RELA_TEXT_NAME_OFFSET,
            kind: 4,
            offset: layout.rela_offset,
            size: layout.rela_size,
            link: SYMTAB_SECTION_INDEX,
            info: u32::from(TEXT_SECTION_INDEX),
            alignment: 8,
            entry_size: RELA_SIZE,
            ..SectionHeader::default()
        },
    );
    write_section_header(
        &mut object,
        &SectionHeader {
            name: SYMTAB_NAME_OFFSET,
            kind: 2,
            offset: layout.symtab_offset,
            size: layout.symtab_size,
            link: STRTAB_SECTION_INDEX,
            info: 1,
            alignment: 8,
            entry_size: SYMBOL_SIZE,
            ..SectionHeader::default()
        },
    );
    write_section_header(
        &mut object,
        &SectionHeader {
            name: STRTAB_NAME_OFFSET,
            kind: 3,
            offset: layout.strtab_offset,
            size: layout.strtab_size,
            alignment: 1,
            ..SectionHeader::default()
        },
    );
    write_section_header(
        &mut object,
        &SectionHeader {
            name: SHSTRTAB_NAME_OFFSET,
            kind: 3,
            offset: layout.shstrtab_offset,
            size: u64::try_from(SHSTRTAB.len()).unwrap(),
            alignment: 1,
            ..SectionHeader::default()
        },
    );

    debug_assert_eq!(object.len(), output_size);
    Ok(object)
}

fn checked_add(left: u64, right: u64, field: &'static str) -> Result<u64, ElfError> {
    left.checked_add(right)
        .ok_or(ElfError::FieldOverflow { field })
}

fn align_offset(offset: u64, alignment: u64, field: &'static str) -> Result<u64, ElfError> {
    debug_assert!(alignment.is_power_of_two());
    let padding = (alignment - offset % alignment) % alignment;
    checked_add(offset, padding, field)
}

fn symbol_name_offset(offset: u64) -> Result<u32, ElfError> {
    u32::try_from(offset).map_err(|_| ElfError::FieldOverflow {
        field: "symbol name offset",
    })
}

fn pad_to(object: &mut Vec<u8>, offset: u64) {
    let offset = usize::try_from(offset).expect("the validated object size contains this offset");
    debug_assert!(object.len() <= offset);
    object.resize(offset, 0);
}

fn write_elf_header(object: &mut Vec<u8>, section_headers_offset: u64) {
    object.extend_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    push_u16(object, 1);
    push_u16(object, 62);
    push_u32(object, 1);
    push_u64(object, 0);
    push_u64(object, 0);
    push_u64(object, section_headers_offset);
    push_u32(object, 0);
    push_u16(object, ELF_HEADER_SIZE as u16);
    push_u16(object, 0);
    push_u16(object, 0);
    push_u16(object, SECTION_HEADER_SIZE as u16);
    push_u16(object, SECTION_COUNT as u16);
    push_u16(object, SHSTRTAB_SECTION_INDEX);
}

fn write_object_symbol(object: &mut Vec<u8>, name: u32, symbol: &ObjectSymbol) {
    push_u32(object, name);
    match symbol {
        ObjectSymbol::Defined(symbol) => {
            object.push(0x12);
            object.push(0);
            push_u16(object, TEXT_SECTION_INDEX);
            push_u64(object, symbol.offset);
            push_u64(object, symbol.size);
        }
        ObjectSymbol::Undefined(_) => {
            object.push(0x10);
            object.push(0);
            push_u16(object, 0);
            push_u64(object, 0);
            push_u64(object, 0);
        }
    }
}

fn write_section_header(object: &mut Vec<u8>, header: &SectionHeader) {
    push_u32(object, header.name);
    push_u32(object, header.kind);
    push_u64(object, header.flags);
    push_u64(object, 0);
    push_u64(object, header.offset);
    push_u64(object, header.size);
    push_u32(object, header.link);
    push_u32(object, header.info);
    push_u64(object, header.alignment);
    push_u64(object, header.entry_size);
}

fn push_u16(object: &mut Vec<u8>, value: u16) {
    object.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(object: &mut Vec<u8>, value: u32) {
    object.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(object: &mut Vec<u8>, value: u64) {
    object.extend_from_slice(&value.to_le_bytes());
}

fn push_i64(object: &mut Vec<u8>, value: i64) {
    object.extend_from_slice(&value.to_le_bytes());
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
    fn writes_rela_sections_symbols_and_symbol_indices() {
        let text = [0_u8; 16];
        let symbols = [
            ObjectSymbol::defined("main", 0, 16),
            ObjectSymbol::undefined("external"),
            ObjectSymbol::defined("helper", 12, 4),
        ];
        let relocations = [
            Relocation::new(1, 2, R_X86_64_PC32, -4),
            Relocation::new(5, 1, R_X86_64_PLT32, -4),
        ];
        let object = write_object_with_relocations(&text, &symbols, &relocations).unwrap();

        assert_eq!(read_u16(&object, 60), 6);
        assert_eq!(read_u16(&object, 62), 5);
        let section_headers_offset = read_u64(&object, 40);
        assert_eq!(section_headers_offset % 8, 0);
        assert_eq!(
            object.len() as u64,
            section_headers_offset + SECTION_COUNT * SECTION_HEADER_SIZE
        );

        let text_header = section(&object, 1);
        assert_eq!(read_u32(text_header, 0), TEXT_NAME_OFFSET);
        assert_eq!(read_u64(text_header, 32), 16);
        assert_eq!(&object[64..80], &text);

        let rela_header = section(&object, RELA_TEXT_SECTION_INDEX.into());
        assert_eq!(read_u32(rela_header, 0), RELA_TEXT_NAME_OFFSET);
        assert_eq!(read_u32(rela_header, 4), 4);
        assert_eq!(read_u32(rela_header, 40), SYMTAB_SECTION_INDEX);
        assert_eq!(read_u32(rela_header, 44), u32::from(TEXT_SECTION_INDEX));
        assert_eq!(read_u64(rela_header, 32), 2 * RELA_SIZE);
        assert_eq!(read_u64(rela_header, 48), 8);
        assert_eq!(read_u64(rela_header, 56), RELA_SIZE);
        let rela = section_contents(&object, rela_header);
        assert_eq!(read_u64(rela, 0), 1);
        assert_eq!(read_u64(rela, 8), (3_u64 << 32) | R_X86_64_PC32 as u64);
        assert_eq!(read_i64(rela, 16), -4);
        assert_eq!(read_u64(rela, 24), 5);
        assert_eq!(read_u64(rela, 32), (2_u64 << 32) | R_X86_64_PLT32 as u64);
        assert_eq!(read_i64(rela, 40), -4);

        let symtab_header = section(&object, SYMTAB_SECTION_INDEX as usize);
        assert_eq!(read_u32(symtab_header, 40), STRTAB_SECTION_INDEX);
        assert_eq!(read_u32(symtab_header, 44), 1);
        assert_eq!(read_u64(symtab_header, 32), 4 * SYMBOL_SIZE);
        let symtab_offset = read_u64(symtab_header, 24) as usize;
        assert!(
            object[symtab_offset..symtab_offset + 24]
                .iter()
                .all(|byte| *byte == 0)
        );

        let main = &object[symtab_offset + 24..symtab_offset + 48];
        assert_eq!(read_u32(main, 0), 1);
        assert_eq!(main[4], 0x12);
        assert_eq!(read_u16(main, 6), TEXT_SECTION_INDEX);
        assert_eq!(read_u64(main, 8), 0);
        assert_eq!(read_u64(main, 16), 16);

        let external = &object[symtab_offset + 48..symtab_offset + 72];
        assert_eq!(read_u32(external, 0), 6);
        assert_eq!(external[4], 0x10);
        assert_eq!(read_u16(external, 6), 0);
        assert_eq!(read_u64(external, 8), 0);
        assert_eq!(read_u64(external, 16), 0);

        let helper = &object[symtab_offset + 72..symtab_offset + 96];
        assert_eq!(read_u32(helper, 0), 15);
        assert_eq!(helper[4], 0x12);
        assert_eq!(read_u16(helper, 6), TEXT_SECTION_INDEX);
        assert_eq!(read_u64(helper, 8), 12);
        assert_eq!(read_u64(helper, 16), 4);

        let strtab = section_contents(&object, section(&object, STRTAB_SECTION_INDEX as usize));
        assert_eq!(strtab, b"\0main\0external\0helper\0");
        assert_eq!(
            section_contents(&object, section(&object, SHSTRTAB_SECTION_INDEX.into())),
            SHSTRTAB
        );

        for index in 1..SECTION_COUNT as usize {
            let header = section(&object, index);
            let offset = read_u64(header, 24);
            let size = read_u64(header, 32);
            let alignment = read_u64(header, 48);
            assert!(offset + size <= section_headers_offset);
            assert!(alignment.is_power_of_two());
            assert_eq!(offset % alignment, 0);
        }
    }

    #[test]
    fn writes_empty_rela_text_without_relocations() {
        let object = write_object_with_relocations(&[], &[], &[]).unwrap();
        let rela_header = section(&object, RELA_TEXT_SECTION_INDEX.into());
        let symtab_header = section(&object, SYMTAB_SECTION_INDEX as usize);

        assert_eq!(read_u64(rela_header, 24), 64);
        assert_eq!(read_u64(rela_header, 32), 0);
        assert_eq!(read_u32(rela_header, 40), SYMTAB_SECTION_INDEX);
        assert_eq!(read_u32(rela_header, 44), u32::from(TEXT_SECTION_INDEX));
        assert_eq!(read_u64(symtab_header, 32), SYMBOL_SIZE);
        assert_eq!(
            section_contents(&object, section(&object, STRTAB_SECTION_INDEX as usize)),
            b"\0"
        );
    }

    #[test]
    fn accepts_defined_and_undefined_symbols_with_both_relocation_types() {
        for symbol in [
            ObjectSymbol::defined("target", 4, 4),
            ObjectSymbol::undefined("target"),
        ] {
            for relocation_type in [R_X86_64_PC32, R_X86_64_PLT32] {
                let object = write_object_with_relocations(
                    &[0; 8],
                    std::slice::from_ref(&symbol),
                    &[Relocation::new(0, 0, relocation_type, i32::MIN as i64)],
                )
                .unwrap();
                let rela = section_contents(&object, section(&object, 2));
                assert_eq!(read_u64(rela, 8), (1_u64 << 32) | relocation_type as u64);
                assert_eq!(read_i64(rela, 16), i32::MIN as i64);
            }
        }
    }

    #[test]
    fn rejects_invalid_symbol_definitions_and_duplicate_names() {
        assert_eq!(
            write_object_with_relocations(&[0; 4], &[ObjectSymbol::undefined("")], &[]),
            Err(ElfRelocError::Symbol(ElfError::EmptySymbolName {
                index: 0
            }))
        );
        assert_eq!(
            write_object_with_relocations(&[0; 4], &[ObjectSymbol::undefined("bad\0name")], &[]),
            Err(ElfRelocError::Symbol(ElfError::SymbolNameContainsNul {
                index: 0
            }))
        );
        assert_eq!(
            write_object_with_relocations(
                &[0; 4],
                &[
                    ObjectSymbol::defined("target", 0, 4),
                    ObjectSymbol::undefined("target"),
                ],
                &[]
            ),
            Err(ElfRelocError::DuplicateSymbolName { index: 1 })
        );
        assert!(matches!(
            write_object_with_relocations(&[0; 4], &[ObjectSymbol::defined("past_end", 5, 0)], &[]),
            Err(ElfRelocError::Symbol(ElfError::SymbolOutOfBounds { .. }))
        ));
        assert!(matches!(
            write_object_with_relocations(
                &[0; 4],
                &[ObjectSymbol::defined("overflow", u64::MAX, 1)],
                &[]
            ),
            Err(ElfRelocError::Symbol(ElfError::SymbolOutOfBounds { .. }))
        ));
    }

    #[test]
    fn rejects_relocation_offsets_without_a_complete_four_byte_field() {
        let symbols = [ObjectSymbol::undefined("target")];
        for (text_size, offset) in [(0, 0), (3, 0), (4, 1), (8, 5), (8, u64::MAX)] {
            assert_eq!(
                write_object_with_relocations(
                    &vec![0; text_size],
                    &symbols,
                    &[Relocation::new(offset, 0, R_X86_64_PC32, 0)],
                ),
                Err(ElfRelocError::RelocationOffsetOutOfBounds {
                    index: 0,
                    offset,
                    text_size: text_size as u64,
                })
            );
        }

        assert!(
            write_object_with_relocations(
                &[0; 4],
                &symbols,
                &[Relocation::new(0, 0, R_X86_64_PC32, 0)],
            )
            .is_ok()
        );
        assert!(
            write_object_with_relocations(
                &[0; 8],
                &symbols,
                &[Relocation::new(4, 0, R_X86_64_PC32, 0)],
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_addends_outside_the_signed_32_bit_field() {
        let symbols = [ObjectSymbol::undefined("target")];
        for addend in [i32::MIN as i64 - 1, i32::MAX as i64 + 1, i64::MIN, i64::MAX] {
            assert_eq!(
                write_object_with_relocations(
                    &[0; 4],
                    &symbols,
                    &[Relocation::new(0, 0, R_X86_64_PLT32, addend)],
                ),
                Err(ElfRelocError::RelocationAddendOutOfRange { index: 0, addend })
            );
        }
        for addend in [i32::MIN as i64, i32::MAX as i64] {
            assert!(
                write_object_with_relocations(
                    &[0; 4],
                    &symbols,
                    &[Relocation::new(0, 0, R_X86_64_PC32, addend)],
                )
                .is_ok()
            );
        }
    }

    #[test]
    fn rejects_unsupported_relocation_types_explicitly() {
        let symbols = [ObjectSymbol::undefined("target")];
        for relocation_type in [0, 1, 7, 23, u32::MAX] {
            assert_eq!(
                write_object_with_relocations(
                    &[0; 4],
                    &symbols,
                    &[Relocation::new(0, 0, relocation_type, 0)],
                ),
                Err(ElfRelocError::UnsupportedRelocationType {
                    index: 0,
                    relocation_type,
                })
            );
        }
    }

    #[test]
    fn rejects_relocation_symbol_indices_outside_the_symbol_slice() {
        let symbols = [ObjectSymbol::undefined("target")];
        assert_eq!(
            write_object_with_relocations(
                &[0; 4],
                &symbols,
                &[Relocation::new(0, 1, R_X86_64_PC32, 0)],
            ),
            Err(ElfRelocError::RelocationSymbolOutOfBounds {
                index: 0,
                symbol_index: 1,
                symbol_count: 1,
            })
        );
        assert!(matches!(
            write_object_with_relocations(
                &[0; 4],
                &symbols,
                &[Relocation::new(0, usize::MAX, R_X86_64_PC32, 0)],
            ),
            Err(ElfRelocError::RelocationSymbolOutOfBounds { .. })
        ));
    }

    #[test]
    fn detects_layout_overflow() {
        assert_eq!(
            Layout::new(u64::MAX, 0, 0, 1).unwrap_err(),
            ElfError::FieldOverflow {
                field: "relocation section offset"
            }
        );
        assert_eq!(
            Layout::new(0, u64::MAX, 0, 1).unwrap_err(),
            ElfError::FieldOverflow {
                field: "relocation section size"
            }
        );
        assert_eq!(
            Layout::new(0, 0, u64::MAX, 1).unwrap_err(),
            ElfError::FieldOverflow {
                field: "symbol table size"
            }
        );
        assert_eq!(
            Layout::new(0, 0, 0, u64::MAX).unwrap_err(),
            ElfError::FieldOverflow {
                field: "section-name string table offset"
            }
        );
    }

    #[test]
    fn system_readelf_accepts_relocations_when_available() {
        let Some(readelf) = find_readelf() else {
            eprintln!("readelf/llvm-readelf not found; skipping external validation");
            return;
        };
        let object = write_object_with_relocations(
            &[0; 16],
            &[
                ObjectSymbol::defined("main", 0, 16),
                ObjectSymbol::undefined("external"),
                ObjectSymbol::defined("helper", 12, 4),
            ],
            &[
                Relocation::new(1, 2, R_X86_64_PC32, -4),
                Relocation::new(5, 1, R_X86_64_PLT32, -4),
            ],
        )
        .unwrap();
        let file = TempObject::new(&object);
        let output = Command::new(readelf)
            .args(["-W", "-r", "-s", "-S"])
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
            ".rela.text",
            "R_X86_64_PC32",
            "R_X86_64_PLT32",
            "0000000000000001",
            "0000000000000005",
            "helper",
            "external",
            "GLOBAL",
            "UND",
        ] {
            assert!(
                stdout.contains(expected),
                "{readelf} output lacks {expected:?}:\n{stdout}"
            );
        }
        assert!(
            !stderr.to_ascii_lowercase().contains("warning"),
            "{readelf} reported a warning:\n{stderr}"
        );
    }

    fn section(object: &[u8], index: usize) -> &[u8] {
        let table = read_u64(object, 40) as usize;
        let start = table + index * SECTION_HEADER_SIZE as usize;
        &object[start..start + SECTION_HEADER_SIZE as usize]
    }

    fn section_contents<'a>(object: &'a [u8], header: &[u8]) -> &'a [u8] {
        let offset = read_u64(header, 24) as usize;
        let size = read_u64(header, 32) as usize;
        &object[offset..offset + size]
    }

    fn read_u16(bytes: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
    }

    fn read_u32(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    fn read_u64(bytes: &[u8], offset: usize) -> u64 {
        u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
    }

    fn read_i64(bytes: &[u8], offset: usize) -> i64 {
        i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
    }

    fn find_readelf() -> Option<&'static str> {
        ["readelf", "llvm-readelf"].into_iter().find(|command| {
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
                env::temp_dir().join(format!("kfrc-elf-reloc-test-{}-{id}.o", process::id()));
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
