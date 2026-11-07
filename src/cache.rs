use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const ENTRY_MAGIC: &[u8; 8] = b"KFRCACHE";
const ENTRY_VERSION: u32 = 1;
const NO_UNIX_MODE: u32 = u32::MAX;
const MAX_ENTRY_SIZE: u64 = 16 * 1024 * 1024 * 1024;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub enum KeyError {
    Io(io::Error),
    Unsupported(&'static str),
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "could not hash compiler input: {error}"),
            Self::Unsupported(feature) => write!(f, "caching is disabled for {feature}"),
        }
    }
}

impl Error for KeyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Unsupported(_) => None,
        }
    }
}

impl From<io::Error> for KeyError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// A key for one exact rustc invocation, not a reusable cross-crate key.
///
/// `rustc_version` should be the complete output of `rustc -vV`. Every input
/// file, including modules, `include!` files and native libraries, must be
/// listed. Invocations with `--extern` or response files are rejected rather
/// than risking reuse with unhashed dependency artifacts or hidden arguments.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CacheKey(String);

impl CacheKey {
    /// Builds a key using the complete current process environment.
    pub fn new(
        rustc_version: &str,
        args: &[OsString],
        input_files: &[PathBuf],
    ) -> Result<Self, KeyError> {
        let environment: Vec<_> = env::vars_os().collect();
        Self::with_environment(rustc_version, args, input_files, &environment)
    }

    /// Builds a key with an explicit environment, useful for hermetic drivers.
    pub fn with_environment(
        rustc_version: &str,
        args: &[OsString],
        input_files: &[PathBuf],
        environment: &[(OsString, OsString)],
    ) -> Result<Self, KeyError> {
        for arg in args {
            let text = arg.to_string_lossy();
            if text == "--extern" || text.starts_with("--extern=") {
                return Err(KeyError::Unsupported("cross-crate --extern dependencies"));
            }
            if text.starts_with('@') {
                return Err(KeyError::Unsupported("rustc response files"));
            }
        }

        let mut hasher = Sha256::new();
        hasher.update(b"kimi-fast-rustc invocation key\0");
        hash_field(&mut hasher, rustc_version.as_bytes());

        hash_u64(&mut hasher, args.len() as u64);
        for arg in args {
            hash_field(&mut hasher, arg.as_os_str().as_encoded_bytes());
        }

        let mut inputs = Vec::with_capacity(input_files.len());
        for path in input_files {
            let mut file = File::open(path)?;
            let mut content_hash = Sha256::new();
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                content_hash.update(&buffer[..count]);
            }
            inputs.push((
                path.as_os_str().as_encoded_bytes().to_vec(),
                content_hash.finish(),
            ));
        }
        inputs.sort_by(|left, right| left.0.cmp(&right.0));
        hash_u64(&mut hasher, inputs.len() as u64);
        for (path, content_hash) in inputs {
            hash_field(&mut hasher, &path);
            hasher.update(&content_hash);
        }

        let mut environment: Vec<_> = environment
            .iter()
            .map(|(name, value)| {
                (
                    name.as_os_str().as_encoded_bytes().to_vec(),
                    value.as_os_str().as_encoded_bytes().to_vec(),
                )
            })
            .collect();
        environment.sort();
        hash_u64(&mut hasher, environment.len() as u64);
        for (name, value) in environment {
            hash_field(&mut hasher, &name);
            hash_field(&mut hasher, &value);
        }

        Ok(Self(hex(&hasher.finish())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One output file. Paths must be relative to the invocation's output root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Artifact {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub unix_mode: Option<u32>,
}

impl Artifact {
    pub fn new(path: PathBuf, bytes: Vec<u8>) -> Self {
        Self {
            path,
            bytes,
            unix_mode: None,
        }
    }

    pub fn from_file(output_root: &Path, relative_path: &Path) -> io::Result<Self> {
        validate_relative_path(relative_path)?;
        let path = output_root.join(relative_path);
        let metadata = fs::metadata(&path)?;
        #[cfg(unix)]
        let unix_mode = {
            use std::os::unix::fs::PermissionsExt;
            Some(metadata.permissions().mode() & 0o7777)
        };
        #[cfg(not(unix))]
        let unix_mode = None;
        Ok(Self {
            path: relative_path.to_path_buf(),
            bytes: fs::read(path)?,
            unix_mode,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Returns a fully validated entry. Missing, unreadable and corrupt entries
    /// are all cache misses so callers can fall back to running rustc.
    pub fn lookup(&self, key: &CacheKey) -> Option<Vec<Artifact>> {
        let path = self.entry_path(key);
        let metadata = fs::metadata(&path).ok()?;
        if metadata.len() > MAX_ENTRY_SIZE {
            return None;
        }
        decode_entry(&fs::read(path).ok()?, key).ok()
    }

    /// Restores a hit using atomic file replacement. Any error is a miss.
    pub fn restore(&self, key: &CacheKey, output_root: &Path) -> bool {
        let Some(artifacts) = self.lookup(key) else {
            return false;
        };
        for artifact in artifacts {
            let path = output_root.join(&artifact.path);
            if atomic_write(&path, &artifact.bytes, artifact.unix_mode).is_err() {
                return false;
            }
        }
        true
    }

    /// Atomically publishes an entry. Callers should treat an error as a cache
    /// write failure and retain the successful compiler result.
    pub fn store(&self, key: &CacheKey, artifacts: &[Artifact]) -> io::Result<()> {
        let bytes = encode_entry(key, artifacts)?;
        let path = self.entry_path(key);
        atomic_write(&path, &bytes, None)
    }

    pub fn store_files(
        &self,
        key: &CacheKey,
        output_root: &Path,
        relative_paths: &[PathBuf],
    ) -> io::Result<()> {
        let mut artifacts = Vec::with_capacity(relative_paths.len());
        for path in relative_paths {
            artifacts.push(Artifact::from_file(output_root, path)?);
        }
        self.store(key, &artifacts)
    }

    fn entry_path(&self, key: &CacheKey) -> PathBuf {
        self.root.join(&key.0[..2]).join(format!("{}.entry", key.0))
    }
}

fn validate_relative_path(path: &Path) -> io::Result<()> {
    if path.as_os_str().is_empty()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "artifact paths must be nonempty, normal relative paths",
        ));
    }
    Ok(())
}

fn encode_entry(key: &CacheKey, artifacts: &[Artifact]) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    output.extend_from_slice(ENTRY_MAGIC);
    put_u32(&mut output, ENTRY_VERSION);
    output.extend_from_slice(key.0.as_bytes());
    put_u64(&mut output, artifacts.len() as u64);

    let mut paths = HashSet::with_capacity(artifacts.len());
    for artifact in artifacts {
        validate_relative_path(&artifact.path)?;
        if !paths.insert(artifact.path.clone()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "duplicate artifact path",
            ));
        }
        let path = artifact.path.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "artifact paths must be valid UTF-8",
            )
        })?;
        put_field(&mut output, path.as_bytes());
        put_u32(&mut output, artifact.unix_mode.unwrap_or(NO_UNIX_MODE));
        put_field(&mut output, &artifact.bytes);
    }
    let checksum = Sha256::digest(&output);
    output.extend_from_slice(&checksum);
    Ok(output)
}

fn decode_entry(bytes: &[u8], key: &CacheKey) -> io::Result<Vec<Artifact>> {
    if bytes.len() < 32 || Sha256::digest(&bytes[..bytes.len() - 32]) != bytes[bytes.len() - 32..] {
        return Err(invalid_entry("entry checksum mismatch"));
    }
    let mut cursor = Cursor::new(&bytes[..bytes.len() - 32]);
    if cursor.take(8)? != ENTRY_MAGIC {
        return Err(invalid_entry("entry magic mismatch"));
    }
    if cursor.u32()? != ENTRY_VERSION {
        return Err(invalid_entry("unsupported entry version"));
    }
    if cursor.take(64)? != key.0.as_bytes() {
        return Err(invalid_entry("entry key mismatch"));
    }

    let count = usize::try_from(cursor.u64()?)
        .map_err(|_| invalid_entry("artifact count does not fit in memory"))?;
    let mut artifacts = Vec::with_capacity(count.min(4096));
    let mut paths = HashSet::with_capacity(count.min(4096));
    for _ in 0..count {
        let path = String::from_utf8(cursor.field()?.to_vec())
            .map(PathBuf::from)
            .map_err(|_| invalid_entry("artifact path is not UTF-8"))?;
        validate_relative_path(&path)?;
        if !paths.insert(path.clone()) {
            return Err(invalid_entry("duplicate artifact path"));
        }
        let mode = cursor.u32()?;
        let data = cursor.field()?.to_vec();
        artifacts.push(Artifact {
            path,
            bytes: data,
            unix_mode: (mode != NO_UNIX_MODE).then_some(mode),
        });
    }
    if !cursor.is_empty() {
        return Err(invalid_entry("trailing entry data"));
    }
    Ok(artifacts)
}

fn invalid_entry(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn atomic_write(path: &Path, bytes: &[u8], unix_mode: Option<u32>) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "output path has no parent"))?;
    fs::create_dir_all(parent)?;
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "output path has no file name")
    })?;

    for _ in 0..128 {
        let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            ".{}.{}.{}.tmp",
            name.to_string_lossy(),
            std::process::id(),
            sequence
        ));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary);
        let mut file = match file {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };

        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            #[cfg(unix)]
            if let Some(mode) = unix_mode {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(mode & 0o7777))?;
            }
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
        "could not allocate a temporary cache file",
    ))
}

fn hash_u64(hasher: &mut Sha256, value: u64) {
    hasher.update(&value.to_be_bytes());
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hash_u64(hasher, value.len() as u64);
    hasher.update(value);
}

fn put_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn put_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn put_field(output: &mut Vec<u8>, value: &[u8]) {
    put_u64(output, value.len() as u64);
    output.extend_from_slice(value);
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
            return Err(invalid_entry("truncated entry"));
        }
        let (head, tail) = self.bytes.split_at(count);
        self.bytes = tail;
        Ok(head)
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn field(&mut self) -> io::Result<&'a [u8]> {
        let length = usize::try_from(self.u64()?)
            .map_err(|_| invalid_entry("field length does not fit in memory"))?;
        self.take(length)
    }

    fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
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
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "kfrc-cache-test-{}-{nonce}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    fn environment(values: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        values
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect()
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
        assert_eq!(
            hex(&Sha256::digest(&vec![b'a'; 1_000_000])),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn key_covers_version_arguments_contents_and_environment() {
        let temp = TempDir::new();
        let input = temp.path().join("main.rs");
        fs::write(&input, b"fn main() {}\n").unwrap();
        let base_args = args(&["--crate-name", "demo", "--emit", "link"]);
        let base_env = environment(&[("RUSTC_BOOTSTRAP", "0"), ("TARGET", "x86_64")]);
        let base = CacheKey::with_environment(
            "rustc test\nhost: x86_64",
            &base_args,
            std::slice::from_ref(&input),
            &base_env,
        )
        .unwrap();

        let reordered_env = environment(&[("TARGET", "x86_64"), ("RUSTC_BOOTSTRAP", "0")]);
        assert_eq!(
            base,
            CacheKey::with_environment(
                "rustc test\nhost: x86_64",
                &base_args,
                std::slice::from_ref(&input),
                &reordered_env,
            )
            .unwrap()
        );
        assert_ne!(
            base,
            CacheKey::with_environment(
                "rustc other",
                &base_args,
                std::slice::from_ref(&input),
                &base_env
            )
            .unwrap()
        );
        assert_ne!(
            base,
            CacheKey::with_environment(
                "rustc test\nhost: x86_64",
                &args(&["--crate-name", "other", "--emit", "link"]),
                std::slice::from_ref(&input),
                &base_env,
            )
            .unwrap()
        );
        assert_ne!(
            base,
            CacheKey::with_environment(
                "rustc test\nhost: x86_64",
                &base_args,
                std::slice::from_ref(&input),
                &environment(&[("RUSTC_BOOTSTRAP", "1"), ("TARGET", "x86_64")]),
            )
            .unwrap()
        );

        fs::write(&input, b"fn main() { println!(\"changed\"); }\n").unwrap();
        assert_ne!(
            base,
            CacheKey::with_environment(
                "rustc test\nhost: x86_64",
                &base_args,
                &[input],
                &base_env,
            )
            .unwrap()
        );
    }

    #[test]
    fn unsafe_dependency_shortcuts_are_rejected() {
        for invocation in [
            args(&["--extern", "foo=libfoo.rlib"]),
            args(&["--extern=foo=libfoo.rlib"]),
            args(&["@rustc-args"]),
        ] {
            assert!(matches!(
                CacheKey::with_environment("rustc test", &invocation, &[], &[]),
                Err(KeyError::Unsupported(_))
            ));
        }
    }

    #[test]
    fn round_trip_restores_nested_artifacts() {
        let temp = TempDir::new();
        let cache = Cache::new(temp.path().join("cache"));
        let key =
            CacheKey::with_environment("rustc test", &args(&["--emit", "link"]), &[], &[]).unwrap();
        let artifacts = [
            Artifact::new(PathBuf::from("demo"), b"binary".to_vec()),
            Artifact::new(PathBuf::from("nested/demo.d"), b"demo.d: main.rs".to_vec()),
        ];
        cache.store(&key, &artifacts).unwrap();
        assert_eq!(cache.lookup(&key).unwrap(), artifacts);

        let output = temp.path().join("output");
        assert!(cache.restore(&key, &output));
        assert_eq!(fs::read(output.join("demo")).unwrap(), b"binary");
        assert_eq!(
            fs::read(output.join("nested/demo.d")).unwrap(),
            b"demo.d: main.rs"
        );
        assert!(
            fs::read_dir(cache.entry_path(&key).parent().unwrap())
                .unwrap()
                .all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp"))
        );
    }

    #[test]
    fn corrupt_and_missing_entries_fail_open() {
        let temp = TempDir::new();
        let cache = Cache::new(temp.path().join("cache"));
        let key = CacheKey::with_environment("rustc test", &[], &[], &[]).unwrap();
        assert!(cache.lookup(&key).is_none());
        assert!(!cache.restore(&key, &temp.path().join("missing")));

        cache
            .store(&key, &[Artifact::new(PathBuf::from("demo"), vec![1, 2, 3])])
            .unwrap();
        let entry = cache.entry_path(&key);
        let mut bytes = fs::read(&entry).unwrap();
        bytes[20] ^= 0x80;
        fs::write(&entry, bytes).unwrap();
        assert!(cache.lookup(&key).is_none());
        assert!(!cache.restore(&key, &temp.path().join("corrupt")));

        cache
            .store(&key, &[Artifact::new(PathBuf::from("demo"), vec![1, 2, 3])])
            .unwrap();
        let mut bytes = fs::read(&entry).unwrap();
        bytes.truncate(bytes.len() - 10);
        fs::write(&entry, bytes).unwrap();
        assert!(cache.lookup(&key).is_none());
    }

    #[test]
    fn unsafe_artifact_paths_are_not_written() {
        let temp = TempDir::new();
        let cache = Cache::new(temp.path().join("cache"));
        let key = CacheKey::with_environment("rustc test", &[], &[], &[]).unwrap();
        for path in [PathBuf::from("../escape"), PathBuf::from("/absolute")] {
            assert_eq!(
                cache
                    .store(&key, &[Artifact::new(path, Vec::new())])
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn executable_permissions_survive_round_trip() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new();
        let cache = Cache::new(temp.path().join("cache"));
        let key = CacheKey::with_environment("rustc test", &[], &[], &[]).unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("demo"), b"binary").unwrap();
        fs::set_permissions(source.join("demo"), fs::Permissions::from_mode(0o755)).unwrap();
        cache
            .store_files(&key, &source, &[PathBuf::from("demo")])
            .unwrap();

        let output = temp.path().join("output");
        assert!(cache.restore(&key, &output));
        assert_eq!(
            fs::metadata(output.join("demo"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }
}
