//! Direct ELF64 relocatable object emission for native sea-of-nodes functions.
//!
//! This pipeline uses only the in-tree graph lowering, x86_64 encoder, and ELF
//! writer. It does not invoke LLVM or an external assembler. Each object contains
//! one global function symbol covering the machine code in `.text`.
//!
//! The current instruction IR and ELF writer have no relocation support. Objects
//! can therefore contain only self-defined, self-contained straight-line integer
//! functions. Calls to other functions, undefined symbols, and references to
//! external data are unsupported; this module emits an object file, not a linked
//! executable or a general-purpose replacement for a relocating compiler.

use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::elf::{ElfError, FunctionSymbol, write_object as write_elf_object};
use crate::native::{CompileError, compile_graph};
use crate::son::Graph;
use crate::x86::MachineFunction;
use crate::x86_encode::EncodeError;

#[derive(Debug)]
pub enum NativeObjectError {
    Compile(CompileError),
    Encode(EncodeError),
    Elf(ElfError),
    Write { path: PathBuf, source: io::Error },
}

impl fmt::Display for NativeObjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compile(error) => write!(formatter, "native graph compilation failed: {error}"),
            Self::Encode(error) => write!(formatter, "x86_64 encoding failed: {error}"),
            Self::Elf(error) => write!(formatter, "ELF object emission failed: {error}"),
            Self::Write { path, source } => {
                write!(
                    formatter,
                    "failed to write object {}: {source}",
                    path.display()
                )
            }
        }
    }
}

impl Error for NativeObjectError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Compile(error) => Some(error),
            Self::Encode(error) => Some(error),
            Self::Elf(error) => Some(error),
            Self::Write { source, .. } => Some(source),
        }
    }
}

impl From<CompileError> for NativeObjectError {
    fn from(error: CompileError) -> Self {
        Self::Compile(error)
    }
}

impl From<EncodeError> for NativeObjectError {
    fn from(error: EncodeError) -> Self {
        Self::Encode(error)
    }
}

impl From<ElfError> for NativeObjectError {
    fn from(error: ElfError) -> Self {
        Self::Elf(error)
    }
}

/// Encodes one machine function as an ELF64 x86_64 relocatable object in memory.
///
/// The function name becomes a global `STT_FUNC` symbol at `.text` offset zero.
/// No relocation sections or undefined symbols are emitted.
pub fn emit_object(function: &MachineFunction) -> Result<Vec<u8>, NativeObjectError> {
    let code = function.emit_machine_code()?;
    let code_size = u64::try_from(code.len()).map_err(|_| ElfError::FieldOverflow {
        field: "function size",
    })?;
    let symbols = [FunctionSymbol::new(function.name.clone(), 0, code_size)];
    Ok(write_elf_object(&code, &symbols)?)
}

impl MachineFunction {
    /// Emits this function as an ELF64 relocatable object without an assembler.
    /// External references are unsupported because no relocations are emitted.
    pub fn emit_object(&self) -> Result<Vec<u8>, NativeObjectError> {
        emit_object(self)
    }

    /// Writes only an ELF64 relocatable `.o` file; no link step is performed.
    pub fn write_object(&self, path: impl AsRef<Path>) -> Result<(), NativeObjectError> {
        write_object_file(self, path)
    }
}

/// Compiles a graph through machine code directly to an ELF64 relocatable object.
///
/// Only graphs accepted by `native::compile_graph` are supported. In particular,
/// this does not support function calls or other constructs requiring relocations.
pub fn compile_graph_to_object(
    graph: &Graph,
    name: impl Into<String>,
) -> Result<Vec<u8>, NativeObjectError> {
    let function = compile_graph(graph, name)?;
    function.emit_object()
}

/// Compiles a graph and writes only its ELF64 relocatable `.o` file.
///
/// This function never invokes an assembler or linker. The resulting object has
/// one defined function and cannot express calls to arbitrary external functions.
pub fn write_graph_object(
    graph: &Graph,
    name: impl Into<String>,
    path: impl AsRef<Path>,
) -> Result<(), NativeObjectError> {
    let function = compile_graph(graph, name)?;
    function.write_object(path)
}

/// Writes one machine function as an ELF64 relocatable `.o` file without linking.
pub fn write_object_file(
    function: &MachineFunction,
    path: impl AsRef<Path>,
) -> Result<(), NativeObjectError> {
    let path = path.as_ref();
    let object = function.emit_object()?;
    fs::write(path, object).map_err(|source| NativeObjectError::Write {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::process::{self, Command};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::son::BinaryOp;

    fn arithmetic_graph() -> Graph {
        let mut graph = Graph::new();
        let start = graph.start();
        let argument = graph.parameter(0);
        let six = graph.constant(6);
        let product = graph.binary(BinaryOp::Mul, argument, six);
        let seven = graph.constant(7);
        let result = graph.binary(BinaryOp::Add, product, seven);
        graph.return_(start, Some(result));
        graph
    }

    #[test]
    fn graph_object_contains_exact_machine_code_and_function_symbol() {
        let graph = arithmetic_graph();
        let function = compile_graph(&graph, "native_answer").unwrap();
        let code = function.emit_machine_code().unwrap();
        let object = compile_graph_to_object(&graph, "native_answer").unwrap();

        assert_eq!(&object[..4], b"\x7fELF");
        assert_eq!(read_u16(&object, 16), 1, "object must be relocatable");
        assert_eq!(read_u16(&object, 18), 62, "object must target x86_64");
        assert_eq!(&object[64..64 + code.len()], code);

        let section_headers = read_u64(&object, 40) as usize;
        let symbol_header = section_headers + 2 * 64;
        let symbol_offset = read_u64(&object, symbol_header + 24) as usize + 24;
        assert_eq!(object[symbol_offset + 4], 0x12, "global function symbol");
        assert_eq!(read_u16(&object, symbol_offset + 6), 1, ".text section");
        assert_eq!(read_u64(&object, symbol_offset + 8), 0);
        assert_eq!(read_u64(&object, symbol_offset + 16), code.len() as u64);

        let string_header = section_headers + 3 * 64;
        let strings_offset = read_u64(&object, string_header + 24) as usize;
        let strings_size = read_u64(&object, string_header + 32) as usize;
        assert_eq!(
            &object[strings_offset..strings_offset + strings_size],
            b"\0native_answer\0"
        );
    }

    #[test]
    fn writes_only_a_relocatable_object_file() {
        let graph = arithmetic_graph();
        let file = TempObject::empty();
        write_graph_object(&graph, "object_only", &file.0).unwrap();
        let object = fs::read(&file.0).unwrap();
        let expected = compile_graph_to_object(&graph, "object_only").unwrap();

        assert_eq!(object, expected);
        assert_eq!(read_u16(&object, 16), 1, "ET_REL, not an executable");
        assert_eq!(read_u64(&object, 24), 0, "no entry point");
        assert_eq!(read_u64(&object, 32), 0, "no program header table");
        assert_eq!(read_u16(&object, 56), 0, "no program headers");
    }

    #[test]
    fn invalid_function_names_are_not_silently_accepted() {
        assert!(matches!(
            compile_graph_to_object(&arithmetic_graph(), ""),
            Err(NativeObjectError::Elf(ElfError::EmptySymbolName {
                index: 0
            }))
        ));
    }

    #[test]
    fn system_readelf_accepts_graph_object_when_available() {
        let Some(readelf) = find_readelf() else {
            eprintln!("readelf/llvm-readelf not found; skipping external validation");
            return;
        };
        let object = compile_graph_to_object(&arithmetic_graph(), "readelf_native").unwrap();
        let file = TempObject::new(&object);
        let output = Command::new(readelf)
            .args(["-h", "-S", "-s", "-r"])
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
            "REL",
            ".text",
            ".symtab",
            "FUNC",
            "GLOBAL",
            "readelf_native",
        ] {
            assert!(
                stdout.contains(expected),
                "{readelf} output lacks {expected:?}:\n{stdout}"
            );
        }
        assert!(
            !stdout.contains("R_X86_64"),
            "unexpected relocation:\n{stdout}"
        );
    }

    fn find_readelf() -> Option<&'static str> {
        ["readelf", "llvm-readelf"].into_iter().find(|command| {
            Command::new(command)
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success())
        })
    }

    fn read_u16(bytes: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
    }

    fn read_u64(bytes: &[u8], offset: usize) -> u64 {
        u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
    }

    struct TempObject(PathBuf);

    impl TempObject {
        fn empty() -> Self {
            Self(temp_path())
        }

        fn new(contents: &[u8]) -> Self {
            let path = temp_path();
            fs::write(&path, contents).unwrap();
            Self(path)
        }
    }

    impl Drop for TempObject {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    fn temp_path() -> PathBuf {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        env::temp_dir().join(format!("kfrc-native-object-test-{}-{id}.o", process::id()))
    }
}
