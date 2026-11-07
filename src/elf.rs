//! Minimal ELF64 little-endian x86_64 relocatable object emission.

use std::collections::HashSet;
use std::error::Error;
use std::fmt;

const ELF_HEADER_SIZE: u64 = 64;
const SECTION_HEADER_SIZE: u64 = 64;
const SYMBOL_SIZE: u64 = 24;
const SECTION_COUNT: u64 = 5;
const TEXT_SECTION_INDEX: u16 = 1;
const STRTAB_SECTION_INDEX: u32 = 3;
const SHSTRTAB_SECTION_INDEX: u16 = 4;
const SHSTRTAB: &[u8] = b"\0.text\0.symtab\0.strtab\0.shstrtab\0";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FunctionSymbol {
    pub name: String,
    pub offset: u64,
    pub size: u64,
}

impl FunctionSymbol {
    pub fn new(name: impl Into<String>, offset: u64, size: u64) -> Self {
        Self {
            name: name.into(),
            offset,
            size,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ElfError {
    EmptySymbolName {
        index: usize,
    },
    SymbolNameContainsNul {
        index: usize,
    },
    DuplicateSymbolName {
        index: usize,
    },
    SymbolOutOfBounds {
        index: usize,
        offset: u64,
        size: u64,
        text_size: u64,
    },
    FieldOverflow {
        field: &'static str,
    },
    AllocationFailed,
}

impl fmt::Display for ElfError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySymbolName { index } => {
                write!(formatter, "function symbol {index} has an empty name")
            }
            Self::SymbolNameContainsNul { index } => {
                write!(
                    formatter,
                    "function symbol {index} has a NUL byte in its name"
                )
            }
            Self::DuplicateSymbolName { index } => {
                write!(formatter, "function symbol {index} has a duplicate name")
            }
            Self::SymbolOutOfBounds {
                index,
                offset,
                size,
                text_size,
            } => write!(
                formatter,
                "function symbol {index} range {offset}..{} exceeds .text size {text_size}",
                offset.saturating_add(*size)
            ),
            Self::FieldOverflow { field } => {
                write!(formatter, "ELF {field} exceeds its representable range")
            }
            Self::AllocationFailed => formatter.write_str("could not allocate the ELF object"),
        }
    }
}

impl Error for ElfError {}

#[derive(Debug)]
struct Layout {
    text_size: u64,
    symtab_offset: u64,
    symtab_size: u64,
    strtab_offset: u64,
    strtab_size: u64,
    shstrtab_offset: u64,
    shstrtab_size: u64,
    section_headers_offset: u64,
    file_size: u64,
}

impl Layout {
    fn new(text_size: u64, symbol_count: u64, strtab_size: u64) -> Result<Self, ElfError> {
        let text_end = checked_add(ELF_HEADER_SIZE, text_size, "symbol table offset")?;
        let symtab_offset = align_offset(text_end, 8, "symbol table offset")?;
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
            symtab_offset,
            symtab_size,
            strtab_offset,
            strtab_size,
            shstrtab_offset,
            shstrtab_size,
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

/// Writes an ELF64 x86_64 relocatable object containing one `.text` section.
///
/// Every function symbol is global and defined in `.text`; offsets are relative
/// to the start of that section. No relocation sections are emitted.
pub fn write_object(text: &[u8], symbols: &[FunctionSymbol]) -> Result<Vec<u8>, ElfError> {
    let text_size =
        u64::try_from(text.len()).map_err(|_| ElfError::FieldOverflow { field: "text size" })?;
    let symbol_count = u64::try_from(symbols.len()).map_err(|_| ElfError::FieldOverflow {
        field: "symbol count",
    })?;

    let mut names = HashSet::new();
    names
        .try_reserve(symbols.len())
        .map_err(|_| ElfError::AllocationFailed)?;
    let mut name_offsets = Vec::new();
    name_offsets
        .try_reserve_exact(symbols.len())
        .map_err(|_| ElfError::AllocationFailed)?;
    let mut strtab_size = 1_u64;

    for (index, symbol) in symbols.iter().enumerate() {
        if symbol.name.is_empty() {
            return Err(ElfError::EmptySymbolName { index });
        }
        if symbol.name.as_bytes().contains(&0) {
            return Err(ElfError::SymbolNameContainsNul { index });
        }
        if !names.insert(symbol.name.as_str()) {
            return Err(ElfError::DuplicateSymbolName { index });
        }
        if symbol.offset > text_size || symbol.size > text_size - symbol.offset {
            return Err(ElfError::SymbolOutOfBounds {
                index,
                offset: symbol.offset,
                size: symbol.size,
                text_size,
            });
        }

        name_offsets.push(symbol_name_offset(strtab_size)?);
        let name_size = u64::try_from(symbol.name.len()).map_err(|_| ElfError::FieldOverflow {
            field: "string table size",
        })?;
        strtab_size = checked_add(strtab_size, name_size, "string table size")?;
        strtab_size = checked_add(strtab_size, 1, "string table size")?;
    }

    let layout = Layout::new(text_size, symbol_count, strtab_size)?;
    let output_size = usize::try_from(layout.file_size).map_err(|_| ElfError::FieldOverflow {
        field: "object size",
    })?;
    let mut object = Vec::new();
    object
        .try_reserve_exact(output_size)
        .map_err(|_| ElfError::AllocationFailed)?;

    write_elf_header(&mut object, layout.section_headers_offset);
    object.extend_from_slice(text);
    pad_to(&mut object, layout.symtab_offset);

    object.extend_from_slice(&[0; SYMBOL_SIZE as usize]);
    for (symbol, name_offset) in symbols.iter().zip(name_offsets) {
        write_function_symbol(&mut object, name_offset, symbol.offset, symbol.size);
    }
    debug_assert_eq!(object.len() as u64, layout.strtab_offset);

    object.push(0);
    for symbol in symbols {
        object.extend_from_slice(symbol.name.as_bytes());
        object.push(0);
    }
    debug_assert_eq!(object.len() as u64, layout.shstrtab_offset);

    object.extend_from_slice(SHSTRTAB);
    pad_to(&mut object, layout.section_headers_offset);

    write_section_header(&mut object, &SectionHeader::default());
    write_section_header(
        &mut object,
        &SectionHeader {
            name: 1,
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
            name: 7,
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
            name: 15,
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
            name: 23,
            kind: 3,
            offset: layout.shstrtab_offset,
            size: layout.shstrtab_size,
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

fn write_function_symbol(object: &mut Vec<u8>, name: u32, value: u64, size: u64) {
    push_u32(object, name);
    object.push(0x12);
    object.push(0);
    push_u16(object, TEXT_SECTION_INDEX);
    push_u64(object, value);
    push_u64(object, size);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::process::{self, Command};
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn writes_headers_sections_and_global_function_symbols() {
        let text = [0xb8, 42, 0, 0, 0, 0xc3];
        let symbols = [
            FunctionSymbol::new("main", 0, 6),
            FunctionSymbol::new("helper", 6, 0),
        ];
        let object = write_object(&text, &symbols).unwrap();

        assert_eq!(
            &object[..16],
            &[0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(read_u16(&object, 16), 1);
        assert_eq!(read_u16(&object, 18), 62);
        assert_eq!(read_u32(&object, 20), 1);
        assert_eq!(read_u64(&object, 24), 0);
        assert_eq!(read_u64(&object, 32), 0);
        assert_eq!(read_u32(&object, 48), 0);
        assert_eq!(read_u16(&object, 52), 64);
        assert_eq!(read_u16(&object, 54), 0);
        assert_eq!(read_u16(&object, 56), 0);
        assert_eq!(read_u16(&object, 58), 64);
        assert_eq!(read_u16(&object, 60), 5);
        assert_eq!(read_u16(&object, 62), 4);

        let section_headers_offset = read_u64(&object, 40);
        assert_eq!(section_headers_offset % 8, 0);
        assert_eq!(
            object.len() as u64,
            section_headers_offset + SECTION_COUNT * SECTION_HEADER_SIZE
        );
        assert!(section(&object, 0).iter().all(|byte| *byte == 0));

        let text_header = section(&object, 1);
        assert_eq!(read_u32(text_header, 0), 1);
        assert_eq!(read_u32(text_header, 4), 1);
        assert_eq!(read_u64(text_header, 8), 0x6);
        assert_eq!(read_u64(text_header, 16), 0);
        assert_eq!(read_u64(text_header, 24), 64);
        assert_eq!(read_u64(text_header, 32), text.len() as u64);
        assert_eq!(read_u64(text_header, 48), 16);
        assert_eq!(&object[64..64 + text.len()], &text);

        let symtab_header = section(&object, 2);
        let symtab_offset = read_u64(symtab_header, 24) as usize;
        assert_eq!(read_u32(symtab_header, 0), 7);
        assert_eq!(read_u32(symtab_header, 4), 2);
        assert_eq!(symtab_offset as u64 % 8, 0);
        assert_eq!(read_u64(symtab_header, 32), 3 * SYMBOL_SIZE);
        assert_eq!(read_u32(symtab_header, 40), 3);
        assert_eq!(read_u32(symtab_header, 44), 1);
        assert_eq!(read_u64(symtab_header, 48), 8);
        assert_eq!(read_u64(symtab_header, 56), SYMBOL_SIZE);
        assert!(
            object[symtab_offset..symtab_offset + 24]
                .iter()
                .all(|byte| *byte == 0)
        );

        let main = &object[symtab_offset + 24..symtab_offset + 48];
        assert_eq!(read_u32(main, 0), 1);
        assert_eq!(main[4], 0x12);
        assert_eq!(main[5], 0);
        assert_eq!(read_u16(main, 6), 1);
        assert_eq!(read_u64(main, 8), 0);
        assert_eq!(read_u64(main, 16), 6);

        let helper = &object[symtab_offset + 48..symtab_offset + 72];
        assert_eq!(read_u32(helper, 0), 6);
        assert_eq!(helper[4], 0x12);
        assert_eq!(read_u16(helper, 6), 1);
        assert_eq!(read_u64(helper, 8), 6);
        assert_eq!(read_u64(helper, 16), 0);

        let strtab_header = section(&object, 3);
        let strtab = section_contents(&object, strtab_header);
        assert_eq!(read_u32(strtab_header, 0), 15);
        assert_eq!(read_u32(strtab_header, 4), 3);
        assert_eq!(read_u64(strtab_header, 48), 1);
        assert_eq!(strtab, b"\0main\0helper\0");

        let shstrtab_header = section(&object, 4);
        assert_eq!(read_u32(shstrtab_header, 0), 23);
        assert_eq!(read_u32(shstrtab_header, 4), 3);
        assert_eq!(read_u64(shstrtab_header, 48), 1);
        assert_eq!(section_contents(&object, shstrtab_header), SHSTRTAB);

        for index in 1..5 {
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
    fn writes_an_empty_text_section_without_symbols() {
        let object = write_object(&[], &[]).unwrap();
        let text_header = section(&object, 1);
        let symtab_header = section(&object, 2);

        assert_eq!(read_u64(text_header, 24), 64);
        assert_eq!(read_u64(text_header, 32), 0);
        assert_eq!(read_u64(symtab_header, 32), SYMBOL_SIZE);
        assert_eq!(section_contents(&object, section(&object, 3)), b"\0");
        assert_eq!(read_u64(&object, 40) % 8, 0);
    }

    #[test]
    fn rejects_invalid_and_duplicate_symbol_names() {
        assert_eq!(
            write_object(&[0xc3], &[FunctionSymbol::new("", 0, 1)]),
            Err(ElfError::EmptySymbolName { index: 0 })
        );
        assert_eq!(
            write_object(&[0xc3], &[FunctionSymbol::new("bad\0name", 0, 1)]),
            Err(ElfError::SymbolNameContainsNul { index: 0 })
        );
        assert_eq!(
            write_object(
                &[0xc3],
                &[
                    FunctionSymbol::new("main", 0, 1),
                    FunctionSymbol::new("main", 0, 1),
                ],
            ),
            Err(ElfError::DuplicateSymbolName { index: 1 })
        );
    }

    #[test]
    fn rejects_symbol_ranges_outside_text_without_overflowing() {
        let text = [0; 4];
        for symbol in [
            FunctionSymbol::new("past_end", 5, 0),
            FunctionSymbol::new("too_large", 3, 2),
            FunctionSymbol::new("overflow", u64::MAX, 1),
        ] {
            assert!(matches!(
                write_object(&text, &[symbol]),
                Err(ElfError::SymbolOutOfBounds { .. })
            ));
        }

        assert!(write_object(&text, &[FunctionSymbol::new("end", 4, 0)]).is_ok());
        assert!(write_object(&text, &[FunctionSymbol::new("all", 0, 4)]).is_ok());
    }

    #[test]
    fn detects_field_and_layout_overflow() {
        assert_eq!(
            symbol_name_offset(u64::from(u32::MAX) + 1),
            Err(ElfError::FieldOverflow {
                field: "symbol name offset"
            })
        );
        assert_eq!(
            Layout::new(u64::MAX, 0, 1).unwrap_err(),
            ElfError::FieldOverflow {
                field: "symbol table offset"
            }
        );
        assert_eq!(
            Layout::new(0, u64::MAX, 1).unwrap_err(),
            ElfError::FieldOverflow {
                field: "symbol table size"
            }
        );
        assert_eq!(
            Layout::new(0, 0, u64::MAX).unwrap_err(),
            ElfError::FieldOverflow {
                field: "section-name string table offset"
            }
        );
        assert_eq!(
            align_offset(u64::MAX, 8, "section header offset"),
            Err(ElfError::FieldOverflow {
                field: "section header offset"
            })
        );
    }

    #[test]
    fn system_readelf_accepts_the_object_when_available() {
        let Some(readelf) = find_readelf() else {
            eprintln!("readelf/llvm-readelf not found; skipping external validation");
            return;
        };
        let object = write_object(&[0xc3], &[FunctionSymbol::new("main", 0, 1)]).unwrap();
        let file = TempObject::new(&object);
        let output = Command::new(readelf)
            .args(["-h", "-S", "-s"])
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
            "ELF64",
            ".text",
            ".symtab",
            ".strtab",
            ".shstrtab",
            "FUNC",
            "GLOBAL",
            "main",
        ] {
            assert!(
                stdout.contains(expected),
                "{readelf} output lacks {expected:?}:\n{stdout}"
            );
        }
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
            let path = env::temp_dir().join(format!("kfrc-elf-test-{}-{id}.o", process::id()));
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
