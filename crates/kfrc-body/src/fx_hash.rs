//! Deterministic multiply-rotate hasher for compiler-internal tables.
//!
//! This is the Fx hash used by rustc (`rustc-hash` 1.x): one rotate, xor and
//! multiply per word. Keys in kfrc tables are small integers and short symbol
//! strings produced by the compiler itself, so SipHash's resistance to chosen
//! keys buys nothing while costing several times more instructions per lookup.
//! Every table keeps the std `HashMap`/`HashSet` API; only the hasher changes,
//! and compiler output never depends on table iteration order.

use std::hash::{BuildHasherDefault, Hasher};

/// rustc-hash 1.x 64-bit seed.
const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

#[derive(Clone, Copy, Default)]
pub struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline]
    fn add_word(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.add_word(u64::from_le_bytes(chunk.try_into().unwrap()));
        }
        let remainder = chunks.remainder();
        if !remainder.is_empty() {
            let mut tail = 0u64;
            for (index, byte) in remainder.iter().enumerate() {
                tail |= u64::from(*byte) << (8 * index);
            }
            self.add_word(tail);
        }
    }

    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.add_word(u64::from(value));
    }

    #[inline]
    fn write_u16(&mut self, value: u16) {
        self.add_word(u64::from(value));
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.add_word(u64::from(value));
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.add_word(value);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.add_word(value as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

pub type FxBuildHasher = BuildHasherDefault<FxHasher>;
pub type HashMap<K, V> = std::collections::HashMap<K, V, FxBuildHasher>;
pub type HashSet<K> = std::collections::HashSet<K, FxBuildHasher>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::{BuildHasher, Hash};

    fn hash_of(value: impl Hash) -> u64 {
        FxBuildHasher::default().hash_one(value)
    }

    #[test]
    fn hashing_is_deterministic_and_distinguishes_small_keys() {
        assert_eq!(hash_of(7u32), hash_of(7u32));
        assert_ne!(hash_of(7u32), hash_of(8u32));
        assert_ne!(hash_of("ab"), hash_of("ba"));
        // Byte strings of every tail length take the chunked path.
        let keys: Vec<String> = (0..20).map(|length| "x".repeat(length)).collect();
        let distinct: HashSet<u64> = keys.iter().map(hash_of).collect();
        assert_eq!(distinct.len(), keys.len());
    }

    #[test]
    fn tables_keep_the_std_api() {
        let mut map: HashMap<u32, &str> = HashMap::default();
        map.insert(3, "three");
        assert_eq!(map.get(&3), Some(&"three"));
        let set: HashSet<u32> = HashSet::from_iter([1, 2, 2]);
        assert_eq!(set.len(), 2);
    }
}
