//! Content-addressed caching for the direct native ELF object backend.
//!
//! Cache failures are non-fatal: missing, unreadable, oversized and corrupt
//! entries are misses, and a failed cache write still returns the compiled object.
//! Only objects produced by the in-tree native object backends are stored.

use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::thread;

use crate::native_object::{NativeObjectError, compile_graph_to_object};
use crate::native_program::ProgramError;
use crate::parallel_native::{self, ParallelCompileError};
use crate::son::{BinaryOp, CompareOp, Graph, NodeId, NodeKind, UnaryOp};

pub const SUPPORTED_TARGET: &str = "x86_64-unknown-linux-gnu";
pub const SUPPORTED_ABI: &str = "sysv64";

const KEY_DOMAIN: &[u8] = b"kimi-fast-rustc native graph object key\0";
const KEY_VERSION: u32 = 1;
const PROGRAM_KEY_DOMAIN: &[u8] = b"kimi-fast-rustc native sharded program key\0";
const PROGRAM_KEY_VERSION: u32 = 1;
const PROGRAM_SHARD_DOMAIN: &[u8] = b"kimi-fast-rustc native program node shard\0";
const PROGRAM_HASH_SHARD_SLOTS: usize = 256;
const ENTRY_MAGIC: &[u8; 8] = b"KFRNOBJ\0";
const ENTRY_VERSION: u32 = 1;
const MAX_ENTRY_SIZE: u64 = 1024 * 1024 * 1024;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct NativeCacheKey(String);

impl NativeCacheKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedObject {
    pub bytes: Vec<u8>,
    pub cache_hit: bool,
}

#[derive(Debug)]
pub enum NativeCacheError {
    UnsupportedTarget { target: String, abi: String },
    Object(NativeObjectError),
    Program(ProgramError),
    ParallelCompile(ParallelCompileError),
}

impl fmt::Display for NativeCacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedTarget { target, abi } => write!(
                formatter,
                "native object caching supports only {SUPPORTED_TARGET}/{SUPPORTED_ABI}, not {target}/{abi}"
            ),
            Self::Object(error) => error.fmt(formatter),
            Self::Program(error) => error.fmt(formatter),
            Self::ParallelCompile(error) => error.fmt(formatter),
        }
    }
}

impl Error for NativeCacheError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::UnsupportedTarget { .. } => None,
            Self::Object(error) => Some(error),
            Self::Program(error) => Some(error),
            Self::ParallelCompile(error) => Some(error),
        }
    }
}

impl From<NativeObjectError> for NativeCacheError {
    fn from(error: NativeObjectError) -> Self {
        Self::Object(error)
    }
}

impl From<ProgramError> for NativeCacheError {
    fn from(error: ProgramError) -> Self {
        Self::Program(error)
    }
}

impl From<ParallelCompileError> for NativeCacheError {
    fn from(error: ParallelCompileError) -> Self {
        Self::ParallelCompile(error)
    }
}

#[derive(Clone, Copy)]
enum CacheObjectKind {
    Function,
    Program,
}

#[derive(Clone, Debug)]
pub struct NativeCache {
    root: PathBuf,
}

impl NativeCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Hashes the complete graph, including stable node IDs, live and trailing
    /// tombstones, input order and all three input channels. The symbol is
    /// included because it is part of the emitted ELF object.
    pub fn key(
        graph: &Graph,
        symbol: &str,
        target: &str,
        abi: &str,
        backend_version: &str,
    ) -> NativeCacheKey {
        Self::key_with_jobs(graph, symbol, target, abi, backend_version, 1)
    }

    fn key_with_jobs(
        graph: &Graph,
        symbol: &str,
        target: &str,
        abi: &str,
        backend_version: &str,
        jobs: usize,
    ) -> NativeCacheKey {
        let slot_count = graph.node_capacity() as u64;
        if graph.functions.is_empty() {
            let mut hasher = Sha256::new();
            hasher.update(KEY_DOMAIN);
            put_u32_hash(&mut hasher, KEY_VERSION);
            hash_graph_header(&mut hasher, graph, symbol, target, abi, backend_version);
            put_u64_hash(&mut hasher, slot_count);
            hash_node_range(&mut hasher, graph, 0, slot_count as usize);
            return NativeCacheKey(hex(&hasher.finish()));
        }

        let mut hasher = Sha256::new();
        hasher.update(PROGRAM_KEY_DOMAIN);
        put_u32_hash(&mut hasher, PROGRAM_KEY_VERSION);
        hash_graph_header(&mut hasher, graph, symbol, target, abi, backend_version);
        put_u64_hash(&mut hasher, slot_count);
        let shard_count = (slot_count as usize)
            .div_ceil(PROGRAM_HASH_SHARD_SLOTS)
            .max(1);
        put_u64_hash(&mut hasher, shard_count as u64);
        for digest in hash_program_shards(graph, slot_count as usize, shard_count, jobs) {
            hasher.update(&digest);
        }
        NativeCacheKey(hex(&hasher.finish()))
    }

    /// Returns a validated cached ELF object. Every cache read or validation
    /// failure is a miss so the caller can use the native compiler instead.
    pub fn lookup(&self, key: &NativeCacheKey) -> Option<Vec<u8>> {
        self.lookup_entry(key, None)
    }

    fn lookup_kind(&self, key: &NativeCacheKey, kind: CacheObjectKind) -> Option<Vec<u8>> {
        self.lookup_entry(key, Some(kind))
    }

    fn lookup_entry(&self, key: &NativeCacheKey, kind: Option<CacheObjectKind>) -> Option<Vec<u8>> {
        let file = File::open(self.entry_path(key)).ok()?;
        let metadata = file.metadata().ok()?;
        if !metadata.is_file() || metadata.len() > MAX_ENTRY_SIZE {
            return None;
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_ENTRY_SIZE + 1).read_to_end(&mut bytes).ok()?;
        if bytes.len() as u64 > MAX_ENTRY_SIZE {
            return None;
        }
        decode_entry(&bytes, key, kind).ok()
    }

    /// Compiles through the in-tree x86_64 encoder and ELF writer, never LLVM.
    /// Unsupported graphs and targets return errors and do not create entries.
    pub fn get_or_compile(
        &self,
        graph: &Graph,
        symbol: &str,
        target: &str,
        abi: &str,
        backend_version: &str,
    ) -> Result<CachedObject, NativeCacheError> {
        if !graph.functions.is_empty() {
            return self.get_or_compile_program(graph, 1, target, abi, backend_version);
        }
        if target != SUPPORTED_TARGET || abi != SUPPORTED_ABI {
            return Err(NativeCacheError::UnsupportedTarget {
                target: target.to_owned(),
                abi: abi.to_owned(),
            });
        }

        let key = Self::key(graph, symbol, target, abi, backend_version);
        if let Some(bytes) = self.lookup_kind(&key, CacheObjectKind::Function) {
            return Ok(CachedObject {
                bytes,
                cache_hit: true,
            });
        }

        let bytes = compile_graph_to_object(graph, symbol)?;
        let _ = self.store(&key, &bytes, CacheObjectKind::Function);
        Ok(CachedObject {
            bytes,
            cache_hit: false,
        })
    }

    pub fn get_or_compile_program(
        &self,
        graph: &Graph,
        jobs: usize,
        target: &str,
        abi: &str,
        backend_version: &str,
    ) -> Result<CachedObject, NativeCacheError> {
        if target != SUPPORTED_TARGET || abi != SUPPORTED_ABI {
            return Err(NativeCacheError::UnsupportedTarget {
                target: target.to_owned(),
                abi: abi.to_owned(),
            });
        }

        let key = Self::key_with_jobs(graph, "main", target, abi, backend_version, jobs);
        if let Some(bytes) = self.lookup_kind(&key, CacheObjectKind::Program) {
            return Ok(CachedObject {
                bytes,
                cache_hit: true,
            });
        }

        let bytes = parallel_native::compile_graph(graph, jobs)?.emit_object()?;
        let _ = self.store(&key, &bytes, CacheObjectKind::Program);
        Ok(CachedObject {
            bytes,
            cache_hit: false,
        })
    }

    fn store(&self, key: &NativeCacheKey, object: &[u8], kind: CacheObjectKind) -> io::Result<()> {
        if !object_matches_kind(object, kind) {
            return Err(invalid_data("refusing to cache a non-native ELF object"));
        }
        let bytes = encode_entry(key, object);
        atomic_write(&self.entry_path(key), &bytes)
    }

    fn entry_path(&self, key: &NativeCacheKey) -> PathBuf {
        self.root.join(&key.0[..2]).join(format!("{}.o", key.0))
    }
}

fn hash_graph_header(
    hasher: &mut Sha256,
    graph: &Graph,
    symbol: &str,
    target: &str,
    abi: &str,
    backend_version: &str,
) {
    hash_field(hasher, target.as_bytes());
    hash_field(hasher, abi.as_bytes());
    hash_field(hasher, backend_version.as_bytes());
    hash_field(hasher, symbol.as_bytes());
    put_u64_hash(hasher, graph.len() as u64);
    put_u64_hash(hasher, graph.functions.len() as u64);
    for function in &graph.functions {
        hash_field(hasher, function.name.as_bytes());
        put_u32_hash(hasher, function.start.0);
        put_u32_hash(hasher, function.return_node.0);
        put_u64_hash(hasher, function.parameter_count as u64);
    }
}

fn hash_program_shards(
    graph: &Graph,
    slot_count: usize,
    shard_count: usize,
    jobs: usize,
) -> Vec<[u8; 32]> {
    let worker_count = jobs.max(1).min(shard_count);
    if worker_count == 1 {
        return (0..shard_count)
            .map(|shard| hash_program_shard(graph, slot_count, shard))
            .collect();
    }

    let next_shard = AtomicUsize::new(0);
    thread::scope(|scope| {
        let next_shard = &next_shard;
        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            handles.push(scope.spawn(move || {
                let mut results = Vec::new();
                loop {
                    let shard = next_shard.fetch_add(1, Ordering::Relaxed);
                    if shard >= shard_count {
                        break;
                    }
                    results.push((shard, hash_program_shard(graph, slot_count, shard)));
                }
                results
            }));
        }

        let mut digests = vec![[0_u8; 32]; shard_count];
        for handle in handles {
            for (shard, digest) in handle.join().expect("native cache hash worker panicked") {
                digests[shard] = digest;
            }
        }
        digests
    })
}

fn hash_program_shard(graph: &Graph, slot_count: usize, shard: usize) -> [u8; 32] {
    let start = shard * PROGRAM_HASH_SHARD_SLOTS;
    let end = (start + PROGRAM_HASH_SHARD_SLOTS).min(slot_count);
    let mut hasher = Sha256::new();
    hasher.update(PROGRAM_SHARD_DOMAIN);
    put_u64_hash(&mut hasher, start as u64);
    put_u64_hash(&mut hasher, end as u64);
    hash_node_range(&mut hasher, graph, start, end);
    hasher.finish()
}

fn hash_node_range(hasher: &mut Sha256, graph: &Graph, start: usize, end: usize) {
    for index in start..end {
        let id = NodeId(index as u32);
        let Some(node) = graph.node(id) else {
            hasher.update(&[0]);
            continue;
        };
        hasher.update(&[1]);
        put_u32_hash(hasher, id.0);
        hash_node_kind(hasher, node.kind);
        hash_inputs(hasher, 1, &node.control);
        hash_inputs(hasher, 2, &node.data);
        hash_inputs(hasher, 3, &node.memory);
    }
}

fn hash_node_kind(hasher: &mut Sha256, kind: NodeKind) {
    match kind {
        NodeKind::Start => hasher.update(&[1]),
        NodeKind::Const(value) => {
            hasher.update(&[2]);
            hasher.update(&value.to_be_bytes());
        }
        NodeKind::Parameter { index } => {
            hasher.update(&[3]);
            put_u64_hash(hasher, index as u64);
        }
        NodeKind::Unary(operation) => {
            hasher.update(&[4]);
            hasher.update(&[match operation {
                UnaryOp::Neg => 1,
                UnaryOp::Not => 2,
            }]);
        }
        NodeKind::Binary(operation) => {
            hasher.update(&[5]);
            hasher.update(&[match operation {
                BinaryOp::Add => 1,
                BinaryOp::Sub => 2,
                BinaryOp::Mul => 3,
                BinaryOp::Div => 4,
                BinaryOp::Rem => 5,
                BinaryOp::And => 6,
                BinaryOp::Or => 7,
                BinaryOp::Xor => 8,
                BinaryOp::Shl => 9,
                BinaryOp::Shr => 10,
                BinaryOp::UDiv => 11,
                BinaryOp::URem => 12,
                BinaryOp::LShr => 13,
            }]);
        }
        NodeKind::Compare(operation) => {
            hasher.update(&[6]);
            hasher.update(&[match operation {
                CompareOp::Eq => 1,
                CompareOp::Ne => 2,
                CompareOp::Lt => 3,
                CompareOp::Le => 4,
                CompareOp::Gt => 5,
                CompareOp::Ge => 6,
                CompareOp::ULt => 7,
                CompareOp::ULe => 8,
                CompareOp::UGt => 9,
                CompareOp::UGe => 10,
            }]);
        }
        NodeKind::Phi => hasher.update(&[7]),
        NodeKind::Return => hasher.update(&[8]),
        NodeKind::Call(target) => {
            hasher.update(&[12]);
            put_u64_hash(hasher, target as u64);
        }
        NodeKind::If => hasher.update(&[9]),
        NodeKind::Projection(index) => hasher.update(&[10, index]),
        NodeKind::Region => hasher.update(&[11]),
        NodeKind::Jump => hasher.update(&[13]),
    }
}

fn hash_inputs(hasher: &mut Sha256, channel: u8, inputs: &[NodeId]) {
    hasher.update(&[channel]);
    put_u64_hash(hasher, inputs.len() as u64);
    for input in inputs {
        put_u32_hash(hasher, input.0);
    }
}

fn encode_entry(key: &NativeCacheKey, object: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    output.extend_from_slice(ENTRY_MAGIC);
    put_u32(&mut output, ENTRY_VERSION);
    output.extend_from_slice(key.0.as_bytes());
    put_field(&mut output, object);
    let checksum = Sha256::digest(&output);
    output.extend_from_slice(&checksum);
    output
}

fn decode_entry(
    bytes: &[u8],
    key: &NativeCacheKey,
    kind: Option<CacheObjectKind>,
) -> io::Result<Vec<u8>> {
    if bytes.len() < 32 || Sha256::digest(&bytes[..bytes.len() - 32]) != bytes[bytes.len() - 32..] {
        return Err(invalid_data("native cache checksum mismatch"));
    }
    let mut cursor = Cursor::new(&bytes[..bytes.len() - 32]);
    if cursor.take(8)? != ENTRY_MAGIC {
        return Err(invalid_data("native cache magic mismatch"));
    }
    if cursor.u32()? != ENTRY_VERSION {
        return Err(invalid_data("unsupported native cache entry version"));
    }
    if cursor.take(64)? != key.0.as_bytes() {
        return Err(invalid_data("native cache key mismatch"));
    }
    let object = cursor.field()?;
    if !cursor.is_empty() {
        return Err(invalid_data("trailing native cache entry data"));
    }
    let supported = match kind {
        Some(kind) => object_matches_kind(object, kind),
        None => is_supported_object(object) || is_supported_program_object(object),
    };
    if !supported {
        return Err(invalid_data("invalid native ELF object in cache"));
    }
    Ok(object.to_vec())
}

fn object_matches_kind(object: &[u8], kind: CacheObjectKind) -> bool {
    match kind {
        CacheObjectKind::Function => is_supported_object(object),
        CacheObjectKind::Program => is_supported_program_object(object),
    }
}

fn is_supported_object(object: &[u8]) -> bool {
    if object.len() < 64
        || object[..16] != [0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    {
        return false;
    }
    if elf_u16(object, 16) != Some(1)
        || elf_u16(object, 18) != Some(62)
        || elf_u32(object, 20) != Some(1)
        || elf_u64(object, 24) != Some(0)
        || elf_u64(object, 32) != Some(0)
        || elf_u32(object, 48) != Some(0)
        || elf_u16(object, 52) != Some(64)
        || elf_u16(object, 54) != Some(0)
        || elf_u16(object, 56) != Some(0)
        || elf_u16(object, 58) != Some(64)
        || elf_u16(object, 60) != Some(5)
        || elf_u16(object, 62) != Some(4)
    {
        return false;
    }

    let Some(section_offset) = elf_u64(object, 40).and_then(|value| usize::try_from(value).ok())
    else {
        return false;
    };
    let Some(section_end) = section_offset.checked_add(5 * 64) else {
        return false;
    };
    if section_end != object.len() {
        return false;
    }

    let expected_types = [0_u32, 1, 2, 3, 3];
    for (index, expected_type) in expected_types.into_iter().enumerate() {
        let header = section_offset + index * 64;
        if elf_u32(object, header + 4) != Some(expected_type) {
            return false;
        }
        let Some(offset) =
            elf_u64(object, header + 24).and_then(|value| usize::try_from(value).ok())
        else {
            return false;
        };
        let Some(size) = elf_u64(object, header + 32).and_then(|value| usize::try_from(value).ok())
        else {
            return false;
        };
        if offset
            .checked_add(size)
            .is_none_or(|end| end > object.len())
        {
            return false;
        }
    }

    let text = section_offset + 64;
    let symbols = section_offset + 2 * 64;
    if elf_u64(object, text + 24) != Some(64)
        || elf_u32(object, symbols + 40) != Some(3)
        || elf_u32(object, symbols + 44) != Some(1)
        || elf_u64(object, symbols + 32) != Some(48)
        || elf_u64(object, symbols + 56) != Some(24)
    {
        return false;
    }
    let Some(symbol_offset) =
        elf_u64(object, symbols + 24).and_then(|value| usize::try_from(value).ok())
    else {
        return false;
    };
    if object
        .get(symbol_offset..symbol_offset + 24)
        .is_none_or(|null_symbol| null_symbol.iter().any(|byte| *byte != 0))
    {
        return false;
    }
    let function = symbol_offset + 24;
    object.get(function + 4..function + 5) == Some(&[0x12])
        && elf_u16(object, function + 6) == Some(1)
        && elf_u64(object, function + 8) == Some(0)
        && elf_u64(object, function + 16) == elf_u64(object, text + 32)
}

fn is_supported_program_object(object: &[u8]) -> bool {
    if object.len() < 64
        || object[..16] != [0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    {
        return false;
    }
    if elf_u16(object, 16) != Some(1)
        || elf_u16(object, 18) != Some(62)
        || elf_u32(object, 20) != Some(1)
        || elf_u64(object, 24) != Some(0)
        || elf_u64(object, 32) != Some(0)
        || elf_u32(object, 48) != Some(0)
        || elf_u16(object, 52) != Some(64)
        || elf_u16(object, 54) != Some(0)
        || elf_u16(object, 56) != Some(0)
        || elf_u16(object, 58) != Some(64)
        || elf_u16(object, 60) != Some(6)
        || elf_u16(object, 62) != Some(5)
    {
        return false;
    }

    let Some(section_offset) = elf_u64(object, 40).and_then(|value| usize::try_from(value).ok())
    else {
        return false;
    };
    let Some(section_end) = section_offset.checked_add(6 * 64) else {
        return false;
    };
    if section_end != object.len() {
        return false;
    }

    let expected_types = [0_u32, 1, 4, 2, 3, 3];
    for (index, expected_type) in expected_types.into_iter().enumerate() {
        let header = section_offset + index * 64;
        if elf_u32(object, header + 4) != Some(expected_type) {
            return false;
        }
        let Some(offset) =
            elf_u64(object, header + 24).and_then(|value| usize::try_from(value).ok())
        else {
            return false;
        };
        let Some(size) = elf_u64(object, header + 32).and_then(|value| usize::try_from(value).ok())
        else {
            return false;
        };
        if offset
            .checked_add(size)
            .is_none_or(|end| end > object.len())
        {
            return false;
        }
    }

    let text = section_offset + 64;
    let relocations = section_offset + 2 * 64;
    let symbols = section_offset + 3 * 64;
    if elf_u64(object, text + 24) != Some(64)
        || elf_u32(object, relocations + 40) != Some(3)
        || elf_u32(object, relocations + 44) != Some(1)
        || elf_u64(object, relocations + 56) != Some(24)
        || elf_u32(object, symbols + 40) != Some(4)
        || elf_u32(object, symbols + 44) != Some(1)
        || elf_u64(object, symbols + 56) != Some(24)
    {
        return false;
    }
    let Some(text_size) = elf_u64(object, text + 32) else {
        return false;
    };
    let Some(symbol_offset) =
        elf_u64(object, symbols + 24).and_then(|value| usize::try_from(value).ok())
    else {
        return false;
    };
    let Some(symbol_size) =
        elf_u64(object, symbols + 32).and_then(|value| usize::try_from(value).ok())
    else {
        return false;
    };
    if symbol_size < 48 || symbol_size % 24 != 0 {
        return false;
    }
    let symbol_count = symbol_size / 24;
    if object
        .get(symbol_offset..symbol_offset + 24)
        .is_none_or(|null_symbol| null_symbol.iter().any(|byte| *byte != 0))
    {
        return false;
    }
    let mut defined_symbols = 0;
    for index in 1..symbol_count {
        let symbol = symbol_offset + index * 24;
        match object.get(symbol + 4..symbol + 5) {
            Some(&[0x12]) => {
                let Some(value) = elf_u64(object, symbol + 8) else {
                    return false;
                };
                let Some(size) = elf_u64(object, symbol + 16) else {
                    return false;
                };
                if elf_u16(object, symbol + 6) != Some(1)
                    || value > text_size
                    || size > text_size - value
                {
                    return false;
                }
                defined_symbols += 1;
            }
            Some(&[0x10]) => {
                if elf_u16(object, symbol + 6) != Some(0)
                    || elf_u64(object, symbol + 8) != Some(0)
                    || elf_u64(object, symbol + 16) != Some(0)
                {
                    return false;
                }
            }
            _ => return false,
        }
    }
    if defined_symbols == 0 {
        return false;
    }

    let Some(relocation_offset) =
        elf_u64(object, relocations + 24).and_then(|value| usize::try_from(value).ok())
    else {
        return false;
    };
    let Some(relocation_size) =
        elf_u64(object, relocations + 32).and_then(|value| usize::try_from(value).ok())
    else {
        return false;
    };
    if relocation_size % 24 != 0 {
        return false;
    }
    for index in 0..relocation_size / 24 {
        let relocation = relocation_offset + index * 24;
        let Some(offset) = elf_u64(object, relocation) else {
            return false;
        };
        let Some(info) = elf_u64(object, relocation + 8) else {
            return false;
        };
        let Ok(symbol_index) = usize::try_from(info >> 32) else {
            return false;
        };
        if offset > text_size
            || text_size - offset < 4
            || info as u32 != 4
            || symbol_index == 0
            || symbol_index >= symbol_count
        {
            return false;
        }
    }
    true
}

fn elf_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn elf_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn elf_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        bytes.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "cache path has no parent"))?;
    fs::create_dir_all(parent)?;
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "cache path has no file name")
    })?;

    for _ in 0..128 {
        let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            ".{}.{}.{}.tmp",
            name.to_string_lossy(),
            std::process::id(),
            sequence
        ));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        return result;
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a temporary native cache file",
    ))
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    put_u64_hash(hasher, value.len() as u64);
    hasher.update(value);
}

fn put_u32_hash(hasher: &mut Sha256, value: u32) {
    hasher.update(&value.to_be_bytes());
}

fn put_u64_hash(hasher: &mut Sha256, value: u64) {
    hasher.update(&value.to_be_bytes());
}

fn put_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn put_field(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u64).to_be_bytes());
    output.extend_from_slice(value);
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

struct Cursor<'a> {
    bytes: &'a [u8],
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        if self.bytes.len() < count {
            return Err(invalid_data("truncated native cache entry"));
        }
        let (head, tail) = self.bytes.split_at(count);
        self.bytes = tail;
        Ok(head)
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn field(&mut self) -> io::Result<&'a [u8]> {
        let length = usize::try_from(self.u64()?)
            .map_err(|_| invalid_data("native object length does not fit in memory"))?;
        self.take(length)
    }

    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffered: usize,
    length_bits: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buffer: [0; 64],
            buffered: 0,
            length_bits: 0,
        }
    }

    fn update(&mut self, mut bytes: &[u8]) {
        self.length_bits = self.length_bits.wrapping_add((bytes.len() as u64) * 8);
        if self.buffered != 0 {
            let count = (64 - self.buffered).min(bytes.len());
            self.buffer[self.buffered..self.buffered + count].copy_from_slice(&bytes[..count]);
            self.buffered += count;
            bytes = &bytes[count..];
            if self.buffered == 64 {
                let block = self.buffer;
                self.compress(&block);
                self.buffered = 0;
            } else {
                return;
            }
        }
        while bytes.len() >= 64 {
            self.compress(bytes[..64].try_into().unwrap());
            bytes = &bytes[64..];
        }
        self.buffer[..bytes.len()].copy_from_slice(bytes);
        self.buffered = bytes.len();
    }

    fn finish(mut self) -> [u8; 32] {
        let length_bits = self.length_bits;
        self.update(&[0x80]);
        while self.buffered != 56 {
            self.update(&[0]);
        }
        self.update(&length_bits.to_be_bytes());
        let mut output = [0_u8; 32];
        for (chunk, word) in output.chunks_exact_mut(4).zip(self.state) {
            chunk.copy_from_slice(&word.to_be_bytes());
        }
        output
    }

    fn digest(bytes: &[u8]) -> [u8; 32] {
        let mut hasher = Self::new();
        hasher.update(bytes);
        hasher.finish()
    }

    fn compress(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut words = [0_u32; 64];
        for (word, chunk) in words[..16].iter_mut().zip(block.chunks_exact(4)) {
            *word = u32::from_be_bytes(chunk.try_into().unwrap());
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ (!e & g);
            let first = h
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let second = s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(first);
            d = c;
            c = b;
            b = a;
            a = first.wrapping_add(second);
        }
        for (state, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *state = state.wrapping_add(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::process;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::native::{CompileError, UnsupportedReason};

    const BACKEND: &str = "native-x86_64-elf-test-v1";

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "kfrc-native-cache-test-{}-{nonce}-{sequence}",
                process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn graph() -> Graph {
        let mut graph = Graph::new();
        let start = graph.start();
        let argument = graph.parameter(0);
        let six = graph.constant(6);
        let seven = graph.constant(7);
        let product = graph.binary(BinaryOp::Mul, argument, six);
        let result = graph.binary(BinaryOp::Add, product, seven);
        graph.return_(start, Some(result));
        graph
    }

    fn key(graph: &Graph) -> NativeCacheKey {
        NativeCache::key(graph, "answer", SUPPORTED_TARGET, SUPPORTED_ABI, BACKEND)
    }

    fn program_graph() -> Graph {
        crate::frontend::parse(
            "fn main() -> i64 { return factorial(5); }\n\
             fn factorial(value: i64) -> i64 {\n\
                 if value <= 1 { return 1; }\n\
                 return value * factorial(value - 1);\n\
             }\n",
        )
        .unwrap()
    }

    fn large_program_graph() -> Graph {
        let mut source = String::from("fn main() -> i64 { return function0(40); }\n");
        for index in 0..96 {
            source.push_str(&format!(
                "fn function{index}(value: i64) -> i64 {{\n\
                 let mut result = value;\n\
                 result += {index};\n\
                 result *= 3;\n\
                 result -= 7;\n\
                 return result;\n\
                 }}\n"
            ));
        }
        crate::frontend::parse(&source).unwrap()
    }

    fn program_key(graph: &Graph, backend: &str) -> NativeCacheKey {
        NativeCache::key(graph, "main", SUPPORTED_TARGET, SUPPORTED_ABI, backend)
    }

    #[test]
    fn sha256_matches_standard_vectors() {
        assert_eq!(
            hex(&Sha256::digest(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn graph_nodes_edges_and_holes_change_the_key() {
        let graph = graph();
        let base = key(&graph);

        let mut changed_node = graph.clone();
        changed_node.node_mut(NodeId(2)).unwrap().kind = NodeKind::Const(8);
        assert_ne!(key(&changed_node), base);

        let mut changed_data_edge = graph.clone();
        changed_data_edge
            .node_mut(NodeId(5))
            .unwrap()
            .data
            .swap(0, 1);
        assert_ne!(key(&changed_data_edge), base);

        let mut changed_control_edge = graph.clone();
        let new_start = changed_control_edge.start();
        changed_control_edge.node_mut(NodeId(6)).unwrap().control[0] = new_start;
        assert_ne!(key(&changed_control_edge), base);

        let mut changed_memory_edge = graph.clone();
        changed_memory_edge.node_mut(NodeId(6)).unwrap().memory = vec![NodeId(2)];
        assert_ne!(key(&changed_memory_edge), base);

        let mut with_hole = Graph::new();
        let start = with_hole.start();
        let argument = with_hole.parameter(0);
        let six = with_hole.constant(6);
        let removed = with_hole.constant(99);
        let seven = with_hole.constant(7);
        let product = with_hole.binary(BinaryOp::Mul, argument, six);
        let result = with_hole.binary(BinaryOp::Add, product, seven);
        with_hole.return_(start, Some(result));
        with_hole.remove(removed);
        assert_ne!(key(&with_hole), base);

        let mut with_trailing_hole = graph.clone();
        let removed = with_trailing_hole.constant(99);
        with_trailing_hole.remove(removed);
        assert_ne!(key(&with_trailing_hole), base);
    }

    #[test]
    fn target_abi_backend_and_symbol_are_isolated() {
        let graph = graph();
        let base = key(&graph);
        assert_ne!(
            NativeCache::key(
                &graph,
                "answer",
                "aarch64-unknown-linux-gnu",
                SUPPORTED_ABI,
                BACKEND
            ),
            base
        );
        assert_ne!(
            NativeCache::key(&graph, "answer", SUPPORTED_TARGET, "other-abi", BACKEND),
            base
        );
        assert_ne!(
            NativeCache::key(
                &graph,
                "answer",
                SUPPORTED_TARGET,
                SUPPORTED_ABI,
                "other-backend"
            ),
            base
        );
        assert_ne!(
            NativeCache::key(&graph, "other", SUPPORTED_TARGET, SUPPORTED_ABI, BACKEND),
            base
        );
    }

    #[test]
    fn program_function_names_order_and_call_targets_change_the_key() {
        let graph = program_graph();
        let base = program_key(&graph, BACKEND);

        let mut renamed = graph.clone();
        renamed.functions[1].name = "recursive".to_owned();
        assert_ne!(program_key(&renamed, BACKEND), base);

        let mut reordered = graph.clone();
        reordered.functions.swap(0, 1);
        assert_ne!(program_key(&reordered, BACKEND), base);

        let call = graph
            .ids()
            .find(|id| matches!(graph.node(*id).unwrap().kind, NodeKind::Call(_)))
            .unwrap();
        let mut changed_call = graph.clone();
        changed_call.node_mut(call).unwrap().kind = NodeKind::Call(0);
        assert_ne!(program_key(&changed_call, BACKEND), base);
        assert_ne!(program_key(&graph, "native-x86_64-elf-test-v2"), base);
    }

    #[test]
    fn program_shards_cover_boundaries_and_holes_independent_of_jobs() {
        let mut graph = program_graph();
        for value in 0..(2 * PROGRAM_HASH_SHARD_SLOTS + 17) {
            graph.constant(value as i64);
        }
        let base = program_key(&graph, BACKEND);
        for jobs in [1, 2, 4, 8] {
            assert_eq!(
                NativeCache::key_with_jobs(
                    &graph,
                    "main",
                    SUPPORTED_TARGET,
                    SUPPORTED_ABI,
                    BACKEND,
                    jobs,
                ),
                base,
                "jobs: {jobs}"
            );
        }

        let mut changed_boundary = graph.clone();
        changed_boundary
            .node_mut(NodeId(PROGRAM_HASH_SHARD_SLOTS as u32))
            .unwrap()
            .kind = NodeKind::Const(-1);
        assert_ne!(program_key(&changed_boundary, BACKEND), base);

        let mut changed_last_shard = graph.clone();
        let last = graph.ids().last().unwrap();
        changed_last_shard.node_mut(last).unwrap().kind = NodeKind::Const(-1);
        assert_ne!(program_key(&changed_last_shard, BACKEND), base);

        let mut interior_hole = graph.clone();
        interior_hole.remove(NodeId(PROGRAM_HASH_SHARD_SLOTS as u32));
        assert_ne!(program_key(&interior_hole, BACKEND), base);

        let mut trailing_hole = graph.clone();
        let removed = trailing_hole.constant(-1);
        trailing_hole.remove(removed);
        assert_eq!(trailing_hole.len(), graph.len());
        assert_ne!(program_key(&trailing_hole, BACKEND), base);
    }

    #[test]
    fn large_program_trailing_tombstones_preserve_sharded_cache_semantics() {
        let graph = large_program_graph();
        assert_eq!(graph.functions.len(), 97);
        assert!(graph.node_capacity() > 3 * PROGRAM_HASH_SHARD_SLOTS);
        let base = program_key(&graph, BACKEND);

        let mut trailing_holes = graph.clone();
        for value in 0..(PROGRAM_HASH_SHARD_SLOTS + 17) {
            let removed = trailing_holes.constant(value as i64);
            trailing_holes.remove(removed);
        }
        assert_eq!(trailing_holes.len(), graph.len());
        assert_eq!(
            trailing_holes.node_capacity(),
            graph.node_capacity() + PROGRAM_HASH_SHARD_SLOTS + 17
        );
        let with_trailing_holes = program_key(&trailing_holes, BACKEND);
        assert_ne!(with_trailing_holes, base);

        for jobs in [1, 4] {
            assert_eq!(
                NativeCache::key_with_jobs(
                    &graph,
                    "main",
                    SUPPORTED_TARGET,
                    SUPPORTED_ABI,
                    BACKEND,
                    jobs,
                ),
                base
            );
            assert_eq!(
                NativeCache::key_with_jobs(
                    &trailing_holes,
                    "main",
                    SUPPORTED_TARGET,
                    SUPPORTED_ABI,
                    BACKEND,
                    jobs,
                ),
                with_trailing_holes
            );
        }

        let temp = TempDir::new();
        let cache = NativeCache::new(temp.0.join("cache"));
        let first = cache
            .get_or_compile_program(&graph, 1, SUPPORTED_TARGET, SUPPORTED_ABI, BACKEND)
            .unwrap();
        assert!(!first.cache_hit);
        let hit = cache
            .get_or_compile_program(&graph, 4, SUPPORTED_TARGET, SUPPORTED_ABI, BACKEND)
            .unwrap();
        assert!(hit.cache_hit);
        assert_eq!(hit.bytes, first.bytes);

        let isolated = cache
            .get_or_compile_program(&trailing_holes, 4, SUPPORTED_TARGET, SUPPORTED_ABI, BACKEND)
            .unwrap();
        assert!(!isolated.cache_hit);
        assert_eq!(isolated.bytes, first.bytes);
        let isolated_hit = cache
            .get_or_compile_program(&trailing_holes, 1, SUPPORTED_TARGET, SUPPORTED_ABI, BACKEND)
            .unwrap();
        assert!(isolated_hit.cache_hit);
        assert_eq!(isolated_hit.bytes, first.bytes);
    }

    #[test]
    fn program_hit_matches_every_job_count_and_isolates_backend_versions() {
        let temp = TempDir::new();
        let cache = NativeCache::new(temp.0.join("cache"));
        let graph = program_graph();
        let expected = crate::native_program::compile_graph_to_object(&graph).unwrap();
        let first = cache
            .get_or_compile_program(&graph, 1, SUPPORTED_TARGET, SUPPORTED_ABI, BACKEND)
            .unwrap();
        assert!(!first.cache_hit);
        assert_eq!(first.bytes, expected);

        for jobs in [2, 8] {
            let hit = cache
                .get_or_compile_program(&graph, jobs, SUPPORTED_TARGET, SUPPORTED_ABI, BACKEND)
                .unwrap();
            assert!(hit.cache_hit, "jobs: {jobs}");
            assert_eq!(hit.bytes, expected, "jobs: {jobs}");
        }

        let other_backend = "native-x86_64-elf-test-v2";
        let recompiled = cache
            .get_or_compile_program(&graph, 4, SUPPORTED_TARGET, SUPPORTED_ABI, other_backend)
            .unwrap();
        assert!(!recompiled.cache_hit);
        assert_eq!(recompiled.bytes, expected);
        assert!(cache.lookup(&program_key(&graph, BACKEND)).is_some());
        assert!(cache.lookup(&program_key(&graph, other_backend)).is_some());

        let compatible_api = cache
            .get_or_compile(&graph, "ignored", SUPPORTED_TARGET, SUPPORTED_ABI, BACKEND)
            .unwrap();
        assert!(compatible_api.cache_hit);
        assert_eq!(compatible_api.bytes, expected);
    }

    #[test]
    fn program_cache_never_accepts_a_single_function_object() {
        let temp = TempDir::new();
        let cache = NativeCache::new(temp.0.join("cache"));
        let single_graph = graph();
        let program_graph = program_graph();
        let single = cache
            .get_or_compile(
                &single_graph,
                "answer",
                SUPPORTED_TARGET,
                SUPPORTED_ABI,
                BACKEND,
            )
            .unwrap();
        let program = cache
            .get_or_compile_program(&program_graph, 2, SUPPORTED_TARGET, SUPPORTED_ABI, BACKEND)
            .unwrap();
        let single_key = key(&single_graph);
        let program_key = program_key(&program_graph, BACKEND);
        assert_ne!(single_key, program_key);

        fs::write(
            cache.entry_path(&program_key),
            encode_entry(&program_key, &single.bytes),
        )
        .unwrap();
        let rebuilt_program = cache
            .get_or_compile_program(&program_graph, 2, SUPPORTED_TARGET, SUPPORTED_ABI, BACKEND)
            .unwrap();
        assert!(!rebuilt_program.cache_hit);
        assert_eq!(rebuilt_program.bytes, program.bytes);

        fs::write(
            cache.entry_path(&single_key),
            encode_entry(&single_key, &program.bytes),
        )
        .unwrap();
        let rebuilt_single = cache
            .get_or_compile(
                &single_graph,
                "answer",
                SUPPORTED_TARGET,
                SUPPORTED_ABI,
                BACKEND,
            )
            .unwrap();
        assert!(!rebuilt_single.cache_hit);
        assert_eq!(rebuilt_single.bytes, single.bytes);
    }

    #[test]
    fn program_emit_failures_do_not_create_entries() {
        let temp = TempDir::new();
        let cache = NativeCache::new(temp.0.join("cache"));
        let mut graph = program_graph();
        graph.functions[1].name = graph.functions[0].name.clone();

        let error = cache
            .get_or_compile_program(&graph, 2, SUPPORTED_TARGET, SUPPORTED_ABI, BACKEND)
            .unwrap_err();

        assert!(matches!(
            error,
            NativeCacheError::Program(ProgramError::Elf(
                crate::elf_reloc::ElfRelocError::DuplicateSymbolName { index: 1 }
            ))
        ));
        assert!(cache.lookup(&program_key(&graph, BACKEND)).is_none());
    }

    #[test]
    fn hit_returns_the_identical_real_elf_object() {
        let temp = TempDir::new();
        let cache = NativeCache::new(temp.0.join("cache"));
        let graph = graph();
        let first = cache
            .get_or_compile(&graph, "answer", SUPPORTED_TARGET, SUPPORTED_ABI, BACKEND)
            .unwrap();
        assert!(!first.cache_hit);
        assert_eq!(&first.bytes[..4], b"\x7fELF");

        let second = cache
            .get_or_compile(&graph, "answer", SUPPORTED_TARGET, SUPPORTED_ABI, BACKEND)
            .unwrap();
        assert!(second.cache_hit);
        assert_eq!(second.bytes, first.bytes);
        assert_eq!(cache.lookup(&key(&graph)), Some(first.bytes));

        let other_target = NativeCache::key(
            &graph,
            "answer",
            "x86_64-other-target",
            SUPPORTED_ABI,
            BACKEND,
        );
        assert!(cache.lookup(&other_target).is_none());
        let entry = cache.entry_path(&key(&graph));
        assert!(fs::read_dir(entry.parent().unwrap()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
    }

    #[test]
    fn corrupt_entries_fail_open_and_are_rebuilt() {
        let temp = TempDir::new();
        let cache = NativeCache::new(temp.0.join("cache"));
        let graph = graph();
        let expected = cache
            .get_or_compile(&graph, "answer", SUPPORTED_TARGET, SUPPORTED_ABI, BACKEND)
            .unwrap()
            .bytes;
        let entry = cache.entry_path(&key(&graph));

        let mut bytes = fs::read(&entry).unwrap();
        bytes[80] ^= 0x80;
        fs::write(&entry, bytes).unwrap();
        assert!(cache.lookup(&key(&graph)).is_none());
        let rebuilt = cache
            .get_or_compile(&graph, "answer", SUPPORTED_TARGET, SUPPORTED_ABI, BACKEND)
            .unwrap();
        assert!(!rebuilt.cache_hit);
        assert_eq!(rebuilt.bytes, expected);
        assert!(cache.lookup(&key(&graph)).is_some());

        let mut bytes = fs::read(&entry).unwrap();
        bytes.truncate(bytes.len() - 17);
        fs::write(&entry, bytes).unwrap();
        assert!(cache.lookup(&key(&graph)).is_none());

        let forged = encode_entry(&key(&graph), b"not an ELF object");
        fs::write(&entry, forged).unwrap();
        assert!(cache.lookup(&key(&graph)).is_none());
    }

    #[test]
    fn unsupported_semantics_and_targets_are_not_cached() {
        let temp = TempDir::new();
        let cache = NativeCache::new(temp.0.join("cache"));
        let mut graph = Graph::new();
        let start = graph.start();
        let value = graph.constant(1);
        graph.add_node_with_memory(NodeKind::Return, vec![start], vec![value], vec![value]);
        let error = cache
            .get_or_compile(&graph, "answer", SUPPORTED_TARGET, SUPPORTED_ABI, BACKEND)
            .unwrap_err();
        assert!(matches!(
            error,
            NativeCacheError::Object(NativeObjectError::Compile(CompileError::Unsupported {
                reason: UnsupportedReason::MemoryInputs,
                ..
            }))
        ));
        assert!(cache.lookup(&key(&graph)).is_none());

        assert!(matches!(
            cache.get_or_compile(
                &self::graph(),
                "answer",
                "aarch64-unknown-linux-gnu",
                SUPPORTED_ABI,
                BACKEND
            ),
            Err(NativeCacheError::UnsupportedTarget { .. })
        ));
    }

    #[test]
    fn cache_write_failure_does_not_hide_compile_success() {
        let temp = TempDir::new();
        let file_instead_of_directory = temp.0.join("file");
        fs::write(&file_instead_of_directory, b"not a directory").unwrap();
        let cache = NativeCache::new(file_instead_of_directory);
        let result = cache
            .get_or_compile(&graph(), "answer", SUPPORTED_TARGET, SUPPORTED_ABI, BACKEND)
            .unwrap();
        assert!(!result.cache_hit);
        assert_eq!(&result.bytes[..4], b"\x7fELF");
    }
}
