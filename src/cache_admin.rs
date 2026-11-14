//! Statistics and conservative administration for the native object cache.
//!
//! The cache root is always supplied by the caller. Clearing preserves that
//! root, removes only its children, and never follows a symbolic link while
//! walking the tree.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const ENTRY_MAGIC: &[u8; 8] = b"KFRNOBJ\0";
const ENTRY_VERSION: u32 = 1;
const MAX_ENTRY_SIZE: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CacheStats {
    pub entries: u64,
    pub total_bytes: u64,
    pub corrupt_entries: u64,
}

impl CacheStats {
    pub fn valid_entries(&self) -> u64 {
        self.entries.saturating_sub(self.corrupt_entries)
    }

    pub fn entry_count(&self) -> u64 {
        self.entries
    }

    pub fn to_human(&self) -> String {
        format!(
            "Entries: {}\nTotal size: {} ({} bytes)\nCorrupt entries: {}",
            self.entries,
            human_size(self.total_bytes),
            self.total_bytes,
            self.corrupt_entries
        )
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"entries\":{},\"total_bytes\":{},\"corrupt_entries\":{}}}",
            self.entries, self.total_bytes, self.corrupt_entries
        )
    }

    pub fn format(&self, format: OutputFormat) -> String {
        match format {
            OutputFormat::Human => self.to_human(),
            OutputFormat::Json => self.to_json(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    Human,
    Json,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ClearReport {
    pub removed_files: u64,
    pub removed_directories: u64,
    pub removed_bytes: u64,
}

impl ClearReport {
    pub fn to_human(&self) -> String {
        format!(
            "Removed files: {}\nRemoved directories: {}\nFreed: {} ({} bytes)",
            self.removed_files,
            self.removed_directories,
            human_size(self.removed_bytes),
            self.removed_bytes
        )
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"removed_files\":{},\"removed_directories\":{},\"removed_bytes\":{}}}",
            self.removed_files, self.removed_directories, self.removed_bytes
        )
    }

    pub fn format(&self, format: OutputFormat) -> String {
        match format {
            OutputFormat::Human => self.to_human(),
            OutputFormat::Json => self.to_json(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct CacheAdmin {
    root: PathBuf,
}

impl CacheAdmin {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn stats(&self) -> io::Result<CacheStats> {
        collect_stats(&self.root)
    }

    pub fn clear(&self) -> io::Result<ClearReport> {
        clear(&self.root)
    }
}

/// Collects statistics for regular `.o` files in native cache prefix directories.
/// Corrupt entries are included in `entries` and `total_bytes`.
pub fn collect_stats(root: impl AsRef<Path>) -> io::Result<CacheStats> {
    let Some(root) = validated_root(root.as_ref())? else {
        return Ok(CacheStats::default());
    };
    let mut stats = CacheStats::default();

    for prefix in fs::read_dir(&root)? {
        let prefix = prefix?;
        let prefix_name = prefix.file_name();
        if !is_hex_prefix(prefix_name.as_encoded_bytes()) {
            continue;
        }
        let prefix_path = prefix.path();
        let metadata = match fs::symlink_metadata(&prefix_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            continue;
        }

        let entries = match fs::read_dir(&prefix_path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                continue;
            }

            let file_name = entry.file_name();
            let file_name = file_name.as_encoded_bytes();
            let Some(key) = file_name.strip_suffix(b".o") else {
                continue;
            };
            stats.entries = stats.entries.checked_add(1).ok_or_else(stats_overflow)?;
            stats.total_bytes = stats
                .total_bytes
                .checked_add(metadata.len())
                .ok_or_else(stats_overflow)?;

            let layout_valid = key.len() == 64
                && key.iter().all(|byte| byte.is_ascii_hexdigit())
                && key[..2] == *prefix_name.as_encoded_bytes();
            if !layout_valid || !valid_entry(&path, key, metadata.len()) {
                stats.corrupt_entries = stats
                    .corrupt_entries
                    .checked_add(1)
                    .ok_or_else(stats_overflow)?;
            }
        }
    }
    Ok(stats)
}

pub fn stats(root: impl AsRef<Path>) -> io::Result<CacheStats> {
    collect_stats(root)
}

/// Removes the contents of an explicitly supplied cache root, not the root itself.
/// Symbolic links are unlinked as links; their targets are never traversed.
pub fn clear(root: impl AsRef<Path>) -> io::Result<ClearReport> {
    let Some(root) = validated_root(root.as_ref())? else {
        return Ok(ClearReport::default());
    };
    let mut report = ClearReport::default();
    clear_directory(&root, &mut report)?;
    Ok(report)
}

pub fn clear_cache(root: impl AsRef<Path>) -> io::Result<ClearReport> {
    clear(root)
}

pub fn format_stats(stats: &CacheStats, format: OutputFormat) -> String {
    stats.format(format)
}

fn validated_root(root: &Path) -> io::Result<Option<PathBuf>> {
    if root.as_os_str().is_empty() {
        return Err(invalid_input("native cache root must not be empty"));
    }
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() {
        return Err(invalid_input(
            "native cache root must not be a symbolic link",
        ));
    }
    if !metadata.file_type().is_dir() {
        return Err(invalid_input("native cache root must be a directory"));
    }

    let root = fs::canonicalize(root)?;
    if root.parent().is_none() {
        return Err(invalid_input(
            "refusing to use a filesystem root as native cache root",
        ));
    }
    Ok(Some(root))
}

fn clear_directory(directory: &Path, report: &mut ClearReport) -> io::Result<()> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let file_type = metadata.file_type();
        if file_type.is_dir() && !file_type.is_symlink() {
            clear_directory(&path, report)?;
            match fs::remove_dir(&path) {
                Ok(()) => {
                    report.removed_directories = report.removed_directories.saturating_add(1);
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        } else {
            match fs::remove_file(&path) {
                Ok(()) => {
                    report.removed_files = report.removed_files.saturating_add(1);
                    report.removed_bytes = report.removed_bytes.saturating_add(metadata.len());
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

fn valid_entry(path: &Path, key: &[u8], length: u64) -> bool {
    if length > MAX_ENTRY_SIZE {
        return false;
    }
    match fs::read(path) {
        Ok(bytes) => bytes.len() as u64 <= MAX_ENTRY_SIZE && decode_entry(&bytes, key).is_ok(),
        Err(_) => false,
    }
}

fn decode_entry(bytes: &[u8], key: &[u8]) -> io::Result<()> {
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
    if cursor.take(64)? != key {
        return Err(invalid_data("native cache key mismatch"));
    }
    let object = cursor.field()?;
    if !cursor.is_empty() {
        return Err(invalid_data("trailing native cache entry data"));
    }
    if !is_supported_object(object) {
        return Err(invalid_data("invalid native ELF object in cache"));
    }
    Ok(())
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
    let Some(symbol_end) = symbol_offset.checked_add(48) else {
        return false;
    };
    if symbol_end > object.len()
        || object[symbol_offset..symbol_offset + 24]
            .iter()
            .any(|byte| *byte != 0)
    {
        return false;
    }
    let function = symbol_offset + 24;
    object.get(function + 4..function + 5) == Some(&[0x12])
        && elf_u16(object, function + 6) == Some(1)
        && elf_u64(object, function + 8) == Some(0)
        && elf_u64(object, function + 16) == elf_u64(object, text + 32)
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

fn is_hex_prefix(name: &[u8]) -> bool {
    name.len() == 2 && name.iter().all(|byte| byte.is_ascii_hexdigit())
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.2} {}", UNITS[unit])
}

fn invalid_input(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn stats_overflow() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "native cache statistics overflow",
    )
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
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    static NEXT_TEST_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_TEST_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "kfrc-cache-admin-test-{}-{timestamp:x}-{id:x}",
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

    fn key(byte: char) -> String {
        std::iter::repeat_n(byte, 64).collect()
    }

    fn entry_path(root: &Path, key: &str) -> PathBuf {
        root.join(&key[..2]).join(format!("{key}.o"))
    }

    fn encode_entry(key: &str, object: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(ENTRY_MAGIC);
        bytes.extend_from_slice(&ENTRY_VERSION.to_be_bytes());
        bytes.extend_from_slice(key.as_bytes());
        bytes.extend_from_slice(&(object.len() as u64).to_be_bytes());
        bytes.extend_from_slice(object);
        let checksum = Sha256::digest(&bytes);
        bytes.extend_from_slice(&checksum);
        bytes
    }

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn valid_object() -> Vec<u8> {
        let section_offset = 128;
        let mut object = vec![0_u8; section_offset + 5 * 64];
        object[..16].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        put_u16(&mut object, 16, 1);
        put_u16(&mut object, 18, 62);
        put_u32(&mut object, 20, 1);
        put_u64(&mut object, 40, section_offset as u64);
        put_u16(&mut object, 52, 64);
        put_u16(&mut object, 58, 64);
        put_u16(&mut object, 60, 5);
        put_u16(&mut object, 62, 4);
        object[64] = 0xc3;

        let text = section_offset + 64;
        put_u32(&mut object, text + 4, 1);
        put_u64(&mut object, text + 24, 64);
        put_u64(&mut object, text + 32, 1);

        let symbols = section_offset + 2 * 64;
        put_u32(&mut object, symbols + 4, 2);
        put_u64(&mut object, symbols + 24, 65);
        put_u64(&mut object, symbols + 32, 48);
        put_u32(&mut object, symbols + 40, 3);
        put_u32(&mut object, symbols + 44, 1);
        put_u64(&mut object, symbols + 56, 24);
        object[65 + 24 + 4] = 0x12;
        put_u16(&mut object, 65 + 24 + 6, 1);
        put_u64(&mut object, 65 + 24 + 16, 1);

        let strings = section_offset + 3 * 64;
        put_u32(&mut object, strings + 4, 3);
        let names = section_offset + 4 * 64;
        put_u32(&mut object, names + 4, 3);
        object
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().fold(String::new(), |mut output, byte| {
            output.push_str(&format!("{byte:02x}"));
            output
        })
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
    fn missing_cache_is_empty_and_stats_include_corrupt_entries() {
        let temp = TempDir::new();
        let root = temp.0.join("cache");
        assert_eq!(stats(&root).unwrap(), CacheStats::default());

        let good_key = key('a');
        let good = encode_entry(&good_key, &valid_object());
        let good_path = entry_path(&root, &good_key);
        fs::create_dir_all(good_path.parent().unwrap()).unwrap();
        fs::write(&good_path, &good).unwrap();

        let bad_key = key('b');
        let bad_path = entry_path(&root, &bad_key);
        fs::create_dir_all(bad_path.parent().unwrap()).unwrap();
        fs::write(&bad_path, b"corrupt").unwrap();

        let malformed = root.join("aa/not-a-key.o");
        fs::write(&malformed, b"bad").unwrap();
        fs::write(root.join("aa/ignored.tmp"), b"ignored").unwrap();
        fs::write(root.join("not-native.o"), b"ignored").unwrap();

        let result = collect_stats(&root).unwrap();
        assert_eq!(result.entries, 3);
        assert_eq!(result.corrupt_entries, 2);
        assert_eq!(result.valid_entries(), 1);
        assert_eq!(result.total_bytes, (good.len() + 7 + 3) as u64);
    }

    #[test]
    fn checksum_and_elf_validation_reject_damaged_entries() {
        let temp = TempDir::new();
        let root = temp.0.join("cache");
        let good_key = key('c');
        let path = entry_path(&root, &good_key);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut bytes = encode_entry(&good_key, &valid_object());
        fs::write(&path, &bytes).unwrap();
        assert_eq!(stats(&root).unwrap().corrupt_entries, 0);

        bytes[80] ^= 0x80;
        fs::write(&path, bytes).unwrap();
        assert_eq!(stats(&root).unwrap().corrupt_entries, 1);

        let invalid_object = encode_entry(&good_key, b"not an ELF object");
        fs::write(&path, invalid_object).unwrap();
        assert_eq!(stats(&root).unwrap().corrupt_entries, 1);
    }

    #[test]
    fn human_and_json_output_expose_the_same_counts() {
        let stats = CacheStats {
            entries: 2,
            total_bytes: 1536,
            corrupt_entries: 1,
        };
        assert_eq!(
            stats.to_json(),
            "{\"entries\":2,\"total_bytes\":1536,\"corrupt_entries\":1}"
        );
        assert!(stats.to_human().contains("Entries: 2"));
        assert!(stats.to_human().contains("1.50 KiB (1536 bytes)"));
        assert!(stats.to_human().contains("Corrupt entries: 1"));
        assert_eq!(format_stats(&stats, OutputFormat::Json), stats.to_json());
        assert_eq!(stats.format(OutputFormat::Human), stats.to_human());
    }

    #[test]
    fn clear_preserves_root_and_removes_nested_contents() {
        let temp = TempDir::new();
        let root = temp.0.join("cache");
        let nested = root.join("aa/nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("entry.o"), b"entry").unwrap();
        fs::write(root.join("temporary"), b"tmp").unwrap();

        let report = clear(&root).unwrap();
        assert_eq!(report.removed_files, 2);
        assert_eq!(report.removed_directories, 2);
        assert_eq!(report.removed_bytes, 8);
        assert!(root.is_dir());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        assert_eq!(stats(&root).unwrap(), CacheStats::default());
        assert!(report.to_json().contains("\"removed_files\":2"));
        assert!(report.to_human().contains("Freed: 8 B"));
    }

    #[test]
    fn clear_rejects_invalid_roots_without_touching_them() {
        let temp = TempDir::new();
        let file = temp.0.join("file");
        fs::write(&file, b"keep").unwrap();
        assert_eq!(
            clear(&file).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(fs::read(&file).unwrap(), b"keep");
        assert_eq!(clear("").unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            clear(temp.0.join("missing")).unwrap(),
            ClearReport::default()
        );
        assert_eq!(clear("/").unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[cfg(unix)]
    #[test]
    fn clear_unlinks_symlinks_without_following_them() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new();
        let outside = temp.0.join("outside");
        fs::create_dir(&outside).unwrap();
        let keep = outside.join("keep.txt");
        fs::write(&keep, b"keep").unwrap();

        let root = temp.0.join("cache");
        fs::create_dir(&root).unwrap();
        symlink(&outside, root.join("linked-directory")).unwrap();
        symlink(&keep, root.join("linked-file")).unwrap();
        let real = root.join("real");
        fs::create_dir(&real).unwrap();
        symlink(&outside, real.join("nested-link")).unwrap();

        clear(&root).unwrap();
        assert_eq!(fs::read(&keep).unwrap(), b"keep");
        assert!(outside.is_dir());
        assert!(root.is_dir());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn root_symlink_is_rejected_and_stats_ignore_entry_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new();
        let outside = temp.0.join("outside");
        let outside_key = key('d');
        let outside_entry = entry_path(&outside, &outside_key);
        fs::create_dir_all(outside_entry.parent().unwrap()).unwrap();
        fs::write(&outside_entry, encode_entry(&outside_key, &valid_object())).unwrap();

        let root = temp.0.join("cache");
        fs::create_dir(&root).unwrap();
        symlink(&outside, root.join("linked-root-content")).unwrap();
        let root_link = temp.0.join("cache-link");
        symlink(&root, &root_link).unwrap();
        assert_eq!(
            clear(&root_link).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(root.join("linked-root-content").is_symlink());

        let prefix = root.join("aa");
        fs::create_dir(&prefix).unwrap();
        let link_key = key('a');
        symlink(&outside_entry, entry_path(&root, &link_key)).unwrap();
        assert_eq!(stats(&root).unwrap(), CacheStats::default());

        clear(&root).unwrap();
        assert!(outside_entry.is_file());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    }
}
