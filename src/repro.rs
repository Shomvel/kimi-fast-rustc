//! Reproducible-build manifests for the direct native ELF backend.

use std::fmt::Write as _;

use crate::elf::{ElfError, FunctionSymbol, write_object};
use crate::native::compile_graph;
use crate::native_cache::{NativeCache, NativeCacheKey};
use crate::native_object::NativeObjectError;
use crate::son::Graph;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeBackendConfig {
    pub target: String,
    pub abi: String,
    pub backend_version: String,
}

impl NativeBackendConfig {
    pub fn new(
        target: impl Into<String>,
        abi: impl Into<String>,
        backend_version: impl Into<String>,
    ) -> Self {
        Self {
            target: target.into(),
            abi: abi.into(),
            backend_version: backend_version.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeBuildManifest {
    pub source_sha256: String,
    /// The complete optimized-graph hash, using the native cache key domain.
    /// The key also covers the function name, target, ABI and backend version.
    pub graph_sha256: String,
    pub target: String,
    pub abi: String,
    pub backend_version: String,
    pub elf_sha256: String,
    pub functions: Vec<FunctionSymbol>,
    pub timestamp_unix_seconds: Option<i64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReproChange {
    Identical,
    InputChanged,
    BackendChanged,
    NonDeterministic,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManifestDiff {
    pub source_changed: bool,
    pub graph_changed: bool,
    pub target_changed: bool,
    pub abi_changed: bool,
    pub backend_version_changed: bool,
    pub elf_changed: bool,
    pub functions_changed: bool,
}

impl ManifestDiff {
    pub const fn input_changed(&self) -> bool {
        self.source_changed || self.graph_changed
    }

    pub const fn backend_changed(&self) -> bool {
        self.target_changed || self.abi_changed || self.backend_version_changed
    }

    pub const fn output_changed(&self) -> bool {
        self.elf_changed || self.functions_changed
    }

    /// Classifies the cause, retaining individual flags for simultaneous changes.
    /// Source changes take precedence, followed by backend changes and other graph
    /// changes. Output differences without either are non-determinism. Timestamps
    /// are deliberately excluded.
    pub const fn classification(&self) -> ReproChange {
        if self.source_changed {
            ReproChange::InputChanged
        } else if self.backend_changed() {
            ReproChange::BackendChanged
        } else if self.graph_changed {
            ReproChange::InputChanged
        } else if self.output_changed() {
            ReproChange::NonDeterministic
        } else {
            ReproChange::Identical
        }
    }
}

impl NativeBuildManifest {
    pub fn from_object(
        source: &[u8],
        graph_key: &NativeCacheKey,
        backend: NativeBackendConfig,
        elf: &[u8],
        functions: Vec<FunctionSymbol>,
        timestamp_unix_seconds: Option<i64>,
    ) -> Self {
        Self {
            source_sha256: sha256_hex(source),
            graph_sha256: graph_key.as_str().to_owned(),
            target: backend.target,
            abi: backend.abi,
            backend_version: backend.backend_version,
            elf_sha256: sha256_hex(elf),
            functions,
            timestamp_unix_seconds,
        }
    }

    /// Compiles one native graph and captures the exact emitted ELF bytes.
    pub fn compile(
        source: &[u8],
        graph: &Graph,
        symbol: &str,
        target: &str,
        abi: &str,
        backend_version: &str,
        timestamp_unix_seconds: Option<i64>,
    ) -> Result<(Self, Vec<u8>), NativeObjectError> {
        let function = compile_graph(graph, symbol)?;
        let code = function.emit_machine_code()?;
        let code_size = u64::try_from(code.len()).map_err(|_| {
            NativeObjectError::Elf(ElfError::FieldOverflow {
                field: "function size",
            })
        })?;
        let functions = vec![FunctionSymbol::new(symbol, 0, code_size)];
        let elf = write_object(&code, &functions)?;
        let backend = NativeBackendConfig::new(target, abi, backend_version);
        let graph_key = NativeCache::key(
            graph,
            symbol,
            &backend.target,
            &backend.abi,
            &backend.backend_version,
        );
        let manifest = Self::from_object(
            source,
            &graph_key,
            backend,
            &elf,
            functions,
            timestamp_unix_seconds,
        );
        Ok((manifest, elf))
    }

    pub fn compare(&self, other: &Self) -> ManifestDiff {
        ManifestDiff {
            source_changed: self.source_sha256 != other.source_sha256,
            graph_changed: self.graph_sha256 != other.graph_sha256,
            target_changed: self.target != other.target,
            abi_changed: self.abi != other.abi,
            backend_version_changed: self.backend_version != other.backend_version,
            elf_changed: self.elf_sha256 != other.elf_sha256,
            functions_changed: self.functions != other.functions,
        }
    }

    pub fn to_json(&self) -> String {
        let mut json = String::from("{\"source_sha256\":");
        push_json_string(&mut json, &self.source_sha256);
        json.push_str(",\"graph_sha256\":");
        push_json_string(&mut json, &self.graph_sha256);
        json.push_str(",\"target\":");
        push_json_string(&mut json, &self.target);
        json.push_str(",\"abi\":");
        push_json_string(&mut json, &self.abi);
        json.push_str(",\"backend_version\":");
        push_json_string(&mut json, &self.backend_version);
        json.push_str(",\"elf_sha256\":");
        push_json_string(&mut json, &self.elf_sha256);
        json.push_str(",\"functions\":[");
        for (index, function) in self.functions.iter().enumerate() {
            if index != 0 {
                json.push(',');
            }
            json.push_str("{\"name\":");
            push_json_string(&mut json, &function.name);
            write!(
                json,
                ",\"offset\":{},\"size\":{}}}",
                function.offset, function.size
            )
            .unwrap();
        }
        json.push_str("],\"timestamp_unix_seconds\":");
        match self.timestamp_unix_seconds {
            Some(timestamp) => write!(json, "{timestamp}").unwrap(),
            None => json.push_str("null"),
        }
        json.push('}');
        json
    }
}

fn push_json_string(json: &mut String, value: &str) {
    json.push('"');
    for character in value.chars() {
        match character {
            '"' => json.push_str("\\\""),
            '\\' => json.push_str("\\\\"),
            '\u{08}' => json.push_str("\\b"),
            '\u{0c}' => json.push_str("\\f"),
            '\n' => json.push_str("\\n"),
            '\r' => json.push_str("\\r"),
            '\t' => json.push_str("\\t"),
            '\u{00}'..='\u{1f}' => {
                write!(json, "\\u{:04x}", u32::from(character)).unwrap();
            }
            _ => json.push(character),
        }
    }
    json.push('"');
}

fn sha256_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
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
    use super::*;
    use crate::native_cache::{SUPPORTED_ABI, SUPPORTED_TARGET};
    use crate::son::BinaryOp;

    const BACKEND: &str = "native-x86_64-elf-test-v1";
    const SOURCE: &[u8] = b"fn answer(x: i64) -> i64 { x * 6 + 7 }";

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

    fn compile(
        graph: &Graph,
        target: &str,
        timestamp_unix_seconds: Option<i64>,
    ) -> (NativeBuildManifest, Vec<u8>) {
        NativeBuildManifest::compile(
            SOURCE,
            graph,
            "answer",
            target,
            SUPPORTED_ABI,
            BACKEND,
            timestamp_unix_seconds,
        )
        .unwrap()
    }

    #[test]
    fn sha256_matches_standard_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn repeated_compilation_is_identical_except_for_optional_time() {
        let (first, first_elf) = compile(&graph(), SUPPORTED_TARGET, Some(100));
        let (second, second_elf) = compile(&graph(), SUPPORTED_TARGET, Some(200));

        assert_eq!(first_elf, second_elf);
        assert_eq!(first.source_sha256, second.source_sha256);
        assert_eq!(first.graph_sha256, second.graph_sha256);
        assert_eq!(first.elf_sha256, second.elf_sha256);
        assert_eq!(first.functions, second.functions);
        assert_eq!(
            first.compare(&second).classification(),
            ReproChange::Identical
        );

        let (third, _) = compile(&graph(), SUPPORTED_TARGET, Some(100));
        assert_eq!(first.to_json(), third.to_json());
    }

    #[test]
    fn optimized_graph_changes_are_input_changes() {
        let (first, _) = compile(&graph(), SUPPORTED_TARGET, None);
        let mut changed = graph();
        changed.node_mut(crate::son::NodeId(2)).unwrap().kind = crate::son::NodeKind::Const(8);
        let (second, _) = compile(&changed, SUPPORTED_TARGET, None);
        let diff = first.compare(&second);

        assert!(diff.graph_changed);
        assert!(diff.input_changed());
        assert!(!diff.backend_changed());
        assert_eq!(diff.classification(), ReproChange::InputChanged);
    }

    #[test]
    fn target_changes_are_backend_changes_even_though_cache_key_changes() {
        let (first, first_elf) = compile(&graph(), SUPPORTED_TARGET, None);
        let target = "aarch64-unknown-linux-gnu";
        let key = NativeCache::key(&graph(), "answer", target, SUPPORTED_ABI, BACKEND);
        let second = NativeBuildManifest::from_object(
            SOURCE,
            &key,
            NativeBackendConfig::new(target, SUPPORTED_ABI, BACKEND),
            &first_elf,
            first.functions.clone(),
            None,
        );
        let diff = first.compare(&second);

        assert!(diff.target_changed);
        assert!(diff.graph_changed);
        assert!(diff.backend_changed());
        assert!(!diff.output_changed());
        assert_eq!(diff.classification(), ReproChange::BackendChanged);
    }

    #[test]
    fn output_changes_with_identical_inputs_are_non_deterministic() {
        let (first, _) = compile(&graph(), SUPPORTED_TARGET, None);
        let mut second = first.clone();
        second.elf_sha256 = sha256_hex(b"different ELF");
        let diff = first.compare(&second);

        assert!(!diff.input_changed());
        assert!(!diff.backend_changed());
        assert!(diff.output_changed());
        assert_eq!(diff.classification(), ReproChange::NonDeterministic);
    }

    #[test]
    fn simultaneous_source_and_backend_changes_retain_all_flags() {
        let (first, _) = compile(&graph(), SUPPORTED_TARGET, None);
        let mut second = first.clone();
        second.source_sha256 = sha256_hex(b"changed source");
        second.backend_version = "native-x86_64-elf-test-v2".to_owned();
        let diff = first.compare(&second);

        assert!(diff.source_changed);
        assert!(diff.backend_version_changed);
        assert_eq!(diff.classification(), ReproChange::InputChanged);
    }

    #[test]
    fn json_contains_all_fields_and_escapes_strings() {
        let graph = graph();
        let key = NativeCache::key(&graph, "answer", SUPPORTED_TARGET, SUPPORTED_ABI, BACKEND);
        let manifest = NativeBuildManifest::from_object(
            SOURCE,
            &key,
            NativeBackendConfig::new("target\"\\\n\u{1}", SUPPORTED_ABI, BACKEND),
            b"\x7fELF",
            vec![FunctionSymbol::new("answer\"\\\n", 0, 42)],
            None,
        );
        let json = manifest.to_json();

        assert!(json.starts_with("{\"source_sha256\":\""));
        assert!(json.contains("\"graph_sha256\":\""));
        assert!(json.contains("\"abi\":\"sysv64\""));
        assert!(json.contains("\"backend_version\":\"native-x86_64-elf-test-v1\""));
        assert!(json.contains("\"elf_sha256\":\""));
        assert!(json.contains("\"functions\":[{\"name\":\"answer\\\"\\\\\\n\""));
        assert!(json.contains("\"offset\":0,\"size\":42"));
        assert!(json.contains("target\\\"\\\\\\n\\u0001"));
        assert!(json.ends_with("\"timestamp_unix_seconds\":null}"));
        assert!(!json.contains('\n'));
    }
}
