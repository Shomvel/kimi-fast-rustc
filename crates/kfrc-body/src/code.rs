//! Encoded machine code as it enters an object file.
//!
//! A machine layer hands over one `EncodedFunction` per emitted symbol. The
//! record is independent of the instruction selector that produced it: bytes,
//! alignment and the relocations that still name other symbols.

/// Final bytes of one function and the symbol references left for the linker.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EncodedFunction {
    pub symbol: String,
    pub bytes: Vec<u8>,
    /// Required start alignment in bytes (a power of two).
    pub align: u64,
    pub relocations: Vec<CodeRelocation>,
}

impl EncodedFunction {
    /// Every symbol this code names, in relocation order with repetitions.
    pub fn references(&self) -> impl Iterator<Item = &str> {
        self.relocations
            .iter()
            .map(|relocation| relocation.target.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeRelocation {
    /// Byte offset of the relocated field from the function start.
    pub offset: u64,
    pub target: String,
    /// The addend recorded in the object file, including any correction for
    /// the distance between the field and the end of its instruction.
    pub addend: i64,
    pub kind: CodeRelocationKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CodeRelocationKind {
    /// 32-bit PC-relative branch target, bound directly or through the PLT.
    Call,
    /// 32-bit PC-relative address of the symbol.
    PcRelative,
    /// 32-bit PC-relative address of the symbol's GOT entry.
    GotPcRelative,
    /// 64-bit absolute address of the symbol.
    Absolute64,
    /// 32-bit offset from the thread pointer (local-exec TLS).
    TlsLocalExec,
    /// 32-bit PC-relative address of the symbol's general-dynamic TLS
    /// descriptor; the following call names `__tls_get_addr`.
    TlsGeneralDynamic,
}
