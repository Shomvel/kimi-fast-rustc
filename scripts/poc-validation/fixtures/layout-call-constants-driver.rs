use layout_call_constants::*;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::ptr::NonNull;

type Guard = unsafe fn(*const u8, *const u8, usize) -> u8;

const GUARDS: [(usize, Guard); 15] = [
    (0, equal_zero),
    (1, equal_one),
    (2, equal_two),
    (3, equal_three),
    (4, equal_four),
    (5, equal_five),
    (6, equal_six),
    (7, equal_seven),
    (8, equal_eight),
    (15, equal_fifteen),
    (16, equal_sixteen),
    (17, equal_seventeen),
    (31, equal_thirty_one),
    (32, equal_thirty_two),
    (33, equal_thirty_three),
];

const LENGTHS: [usize; 26] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 14, 15, 16, 17, 18, 30, 31, 32, 33, 34, 63, 64, 65,
    127, 256, 1023,
];
const CAPACITY: usize = 4096;
const SEED: u64 = 0x7f03_1ace_d09b_6285;

fn next_random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

// Volatile byte loads keep the LLVM-built reference bytewise. Only the first
// different byte determines the ordering sign.
#[inline(never)]
fn byte_order(left: &[u8], right: &[u8]) -> i32 {
    assert_eq!(left.len(), right.len());
    for index in 0..left.len() {
        // The loop bounds and equal lengths prove both initialized byte reads.
        let left_byte = unsafe { left.as_ptr().add(index).read_volatile() };
        let right_byte = unsafe { right.as_ptr().add(index).read_volatile() };
        if left_byte < right_byte {
            return -1;
        }
        if left_byte > right_byte {
            return 1;
        }
    }
    0
}

fn changing_reference(left: &[u8], right: &[u8], lengths: &[usize]) -> u64 {
    let mut result = 0u64;
    for &length in lengths {
        let equal = byte_order(&left[..length], &right[..length]) == 0;
        result = result.rotate_left(5) ^ (length as u64).wrapping_mul(17) ^ u64::from(equal);
    }
    result
}

fn refreshed_reference(left: &[u8], right: &[u8], lengths: &[usize]) -> u64 {
    let mut result = 0u64;
    for &length in lengths {
        let value = if length == 7 {
            u64::from(byte_order(&left[..7], &right[..7]) == 0)
        } else {
            2
        };
        result = result.rotate_left(3) ^ (length as u64).wrapping_mul(13) ^ value;
    }
    result
}

struct Harness {
    corpus: BufWriter<File>,
    checksum: u64,
    pair_cases: usize,
    guard_page_pairs: usize,
    loop_sequences: usize,
    loop_iterations: usize,
    skipped_guard_calls: usize,
    checks: usize,
}

impl Harness {
    fn new(corpus_path: &str) -> Self {
        let mut corpus = BufWriter::new(File::create(corpus_path).expect("create input transcript"));
        corpus
            .write_all(b"kfrc-layout-call-constants-v1\0")
            .expect("write transcript header");
        Self {
            corpus,
            checksum: 0,
            pair_cases: 0,
            guard_page_pairs: 0,
            loop_sequences: 0,
            loop_iterations: 0,
            skipped_guard_calls: 0,
            checks: 0,
        }
    }

    fn number(&mut self, value: usize) {
        self.corpus
            .write_all(&(value as u64).to_le_bytes())
            .expect("write transcript integer");
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.corpus.write_all(bytes).expect("write transcript bytes");
    }

    fn check(&mut self, actual: u64, expected: u64, label: &str, length: usize) {
        assert_eq!(actual, expected, "{label}, length={length}");
        self.checks += 1;
        self.checksum = self.checksum.rotate_left(7)
            ^ actual.wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ length as u64;
    }

    fn pair(&mut self, left: &[u8], right: &[u8], other: &[u8]) {
        let dangling = NonNull::<u8>::dangling().as_ptr().cast_const();
        let pointers = if left.is_empty() {
            // compare_bytes requires non-null aligned pointers even for zero.
            [dangling; 3]
        } else {
            [left.as_ptr(), right.as_ptr(), other.as_ptr()]
        };
        // Slices are live, fully initialized readable ranges of equal length.
        unsafe { self.pair_at(left, right, other, pointers, false) };
    }

    /// The supplied pointers have the same readable initialized contents as
    /// the slices, for their entire lengths, and remain live for all calls.
    /// Empty slices use non-null aligned pointers, which need not be readable.
    unsafe fn pair_at(
        &mut self,
        left: &[u8],
        right: &[u8],
        other: &[u8],
        pointers: [*const u8; 3],
        guarded_pages: bool,
    ) {
        let length = left.len();
        assert_eq!(length, right.len());
        assert_eq!(length, other.len());
        let equal = byte_order(left, right) == 0;
        let other_equal = byte_order(left, other) == 0;
        self.bytes(&[u8::from(guarded_pages)]);
        self.number(length);
        // Record alias relationships without recording process-specific addresses.
        self.bytes(&[u8::from(pointers[0] == pointers[1])
            | (u8::from(pointers[0] == pointers[2]) << 1)
            | (u8::from(pointers[1] == pointers[2]) << 2)]);
        self.bytes(left);
        self.bytes(right);
        self.bytes(other);
        self.pair_cases += 1;
        self.guard_page_pairs += usize::from(guarded_pages);
        let [left_pointer, right_pointer, other_pointer] = pointers;
        for (guard_length, function) in GUARDS {
            let actual = unsafe { function(left_pointer, right_pointer, length) };
            let expected = if length == guard_length { u64::from(equal) } else { 2 };
            self.check(u64::from(actual), expected, "equality guard", length);
        }
        let actual = unsafe { not_equal_guard(left_pointer, right_pointer, length) };
        let expected = if length == 3 { u64::from(!equal) } else { 2 };
        self.check(u64::from(actual), expected, "inequality guard", length);
        let actual = unsafe { inverted_guard(left_pointer, right_pointer, length) };
        let expected = if length == 7 { u64::from(equal) } else { 2 };
        self.check(u64::from(actual), expected, "inverted guard", length);
        let actual = unsafe { two_calls(left_pointer, right_pointer, other_pointer, length) };
        let expected = if length == 7 {
            u64::from(equal) | (u64::from(!other_equal) << 1)
        } else {
            4
        };
        self.check(u64::from(actual), expected, "two calls", length);
        for first_branch in [false, true] {
            let actual = unsafe { sibling_fallback(left_pointer, right_pointer, length, first_branch) };
            let tag = if first_branch && length == 3 {
                0
            } else if !first_branch && length == 16 {
                2
            } else {
                4
            };
            self.check(u64::from(actual), tag + u64::from(equal), "sibling fallback", length);
        }
        let actual = unsafe { merged_fallback(left_pointer, right_pointer, other_pointer, length) };
        let selected = if length == 7 { left } else { right };
        let expected = byte_order(selected, other) == 0;
        self.check(u64::from(actual), u64::from(expected), "merged fallback", length);
        let actual = unsafe { ordering(left_pointer, right_pointer, length) }.signum();
        let expected = byte_order(left, right);
        // Never compare or checksum memcmp's unspecified nonzero magnitude.
        self.check((actual + 1) as u64, (expected + 1) as u64, "ordering sign", length);
    }

    fn loop_case(&mut self, left: &[u8], right: &[u8], lengths: &[usize]) {
        assert_eq!(left.len(), right.len());
        assert!(lengths.iter().all(|&length| length <= left.len()));
        self.bytes(&[2]);
        self.number(left.len());
        self.bytes(&[u8::from(left.as_ptr() == right.as_ptr())]);
        self.bytes(left);
        self.bytes(right);
        self.number(lengths.len());
        for &length in lengths {
            self.number(length);
        }
        self.loop_sequences += 1;
        // Both target functions independently visit the complete sequence.
        self.loop_iterations += 2 * lengths.len();
        // Both full arrays remain initialized and readable for every length.
        let actual = unsafe { changing_lengths(left.as_ptr(), right.as_ptr(), lengths) };
        self.check(actual, changing_reference(left, right, lengths), "changing lengths", lengths.len());
        let actual = unsafe { refreshed_guard(left.as_ptr(), right.as_ptr(), lengths) };
        self.check(actual, refreshed_reference(left, right, lengths), "refreshed guard", lengths.len());
    }

    /// `unreadable` is non-null, but no code on a tested path may dereference it.
    unsafe fn skipped(&mut self, unreadable: *const u8) {
        for (guard_length, function) in GUARDS {
            for length in [guard_length + 1, usize::MAX] {
                self.bytes(&[3]);
                self.number(guard_length);
                self.number(length);
                let actual = unsafe { function(unreadable, unreadable, length) };
                self.check(u64::from(actual), 2, "unreadable untaken guard", length);
                self.skipped_guard_calls += 1;
            }
        }
        self.bytes(&[4]);
        let actual = unsafe { not_equal_guard(unreadable, unreadable, usize::MAX) };
        self.check(u64::from(actual), 2, "unreadable inequality guard", usize::MAX);
        self.skipped_guard_calls += 1;
        let actual = unsafe { inverted_guard(unreadable, unreadable, usize::MAX) };
        self.check(u64::from(actual), 2, "unreadable inverted guard", usize::MAX);
        self.skipped_guard_calls += 1;
        let actual = unsafe { two_calls(unreadable, unreadable, unreadable, usize::MAX) };
        self.check(u64::from(actual), 4, "unreadable two-call guard", usize::MAX);
        self.skipped_guard_calls += 1;
        let lengths = [0, 1, 8, 16, usize::MAX, 3, 33, 0];
        self.number(lengths.len());
        for length in lengths {
            self.number(length);
        }
        let mut expected = 0u64;
        for length in lengths {
            expected = expected.rotate_left(3) ^ (length as u64).wrapping_mul(13) ^ 2;
        }
        // None of these elements is seven, so no compare_bytes call is legal.
        let actual = unsafe { refreshed_guard(unreadable, unreadable, &lengths) };
        self.check(actual, expected, "unreadable refreshed guard", lengths.len());
        self.skipped_guard_calls += 1;
    }
}

#[cfg(target_os = "linux")]
mod guard_pages {
    use super::{Harness, GUARDS};
    use std::ffi::{c_int, c_void};
    use std::ptr::NonNull;

    const PROT_NONE: c_int = 0;
    const PROT_READ: c_int = 1;
    const PROT_WRITE: c_int = 2;
    const MAP_PRIVATE: c_int = 2;
    const MAP_ANONYMOUS: c_int = 0x20;

    extern "C" {
        fn getpagesize() -> c_int;
        fn mmap(address: *mut c_void, length: usize, protection: c_int, flags: c_int, fd: c_int, offset: isize) -> *mut c_void;
        fn mprotect(address: *mut c_void, length: usize, protection: c_int) -> c_int;
        fn munmap(address: *mut c_void, length: usize) -> c_int;
    }

    struct Pages {
        base: *mut u8,
        page_size: usize,
    }

    impl Pages {
        fn new() -> Self {
            // getpagesize has no preconditions and mmap owns a fresh mapping.
            let page_size = unsafe { getpagesize() };
            assert!(page_size > 0);
            let page_size = page_size as usize;
            let address = unsafe {
                mmap(std::ptr::null_mut(), page_size * 2, PROT_READ | PROT_WRITE,
                    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0)
            };
            assert_ne!(address as isize, -1, "mmap: {}", std::io::Error::last_os_error());
            assert!(!address.is_null(), "mmap returned null");
            let pages = Self { base: address.cast(), page_size };
            // This is exactly the page-aligned second half of this mapping.
            let status = unsafe {
                mprotect(pages.base.add(page_size).cast(), page_size, PROT_NONE)
            };
            assert_eq!(status, 0, "mprotect: {}", std::io::Error::last_os_error());
            pages
        }

        fn tail(&mut self, bytes: &[u8]) -> *const u8 {
            assert!(bytes.len() <= self.page_size);
            if bytes.is_empty() {
                return NonNull::<u8>::dangling().as_ptr().cast_const();
            }
            // The destination is wholly in the first, writable page. The
            // complete comparison range ends exactly at the PROT_NONE page.
            let start = unsafe { self.base.add(self.page_size - bytes.len()) };
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), start, bytes.len()) };
            start.cast_const()
        }

        fn unreadable(&self) -> *const u8 {
            // Forming a raw pointer into a protected mapping does not read it.
            unsafe { self.base.add(self.page_size).cast_const() }
        }
    }

    impl Drop for Pages {
        fn drop(&mut self) {
            // This owner releases its complete mapping once; no pointers escape.
            let status = unsafe { munmap(self.base.cast(), self.page_size * 2) };
            assert_eq!(status, 0, "munmap: {}", std::io::Error::last_os_error());
        }
    }

    pub fn run(harness: &mut Harness) {
        let mut left_pages = Pages::new();
        let mut right_pages = Pages::new();
        let mut other_pages = Pages::new();
        for (length, _) in GUARDS {
            let left: Vec<u8> = (0..length).map(|index| (index * 73 + 19) as u8).collect();
            let mut right = left.clone();
            let other: Vec<u8> = (0..length).map(|index| (index * 73 + 92) as u8).collect();
            let left_pointer = left_pages.tail(&left);
            let right_pointer = right_pages.tail(&right);
            let other_pointer = other_pages.tail(&other);
            // All three live mappings contain the full initialized range; no
            // reference is formed for a byte in a protected page.
            unsafe {
                harness.pair_at(&left, &right, &other,
                    [left_pointer, right_pointer, other_pointer], true);
                harness.pair_at(&left, &left, &left,
                    [left_pointer, left_pointer, left_pointer], true);
            }
            for position in 0..length {
                right[position] ^= 0x80;
                let right_pointer = right_pages.tail(&right);
                unsafe {
                    harness.pair_at(&left, &right, &other,
                        [left_pointer, right_pointer, other_pointer], true);
                    harness.pair_at(&right, &left, &other,
                        [right_pointer, left_pointer, other_pointer], true);
                }
                right[position] ^= 0x80;
            }
        }
        // These paths only observe the scalar guard and must not read memory.
        unsafe { harness.skipped(left_pages.unreadable()) };
    }
}

fn main() {
    let corpus_path = std::env::args().nth(1).expect("input transcript output path");
    let mut harness = Harness::new(&corpus_path);
    let mut left = [0u8; CAPACITY + 1];
    for (index, byte) in left.iter_mut().enumerate() {
        *byte = (index * 73 + 19) as u8;
    }
    let mut right = left;
    // Every tested length gets equal, identical-pointer, both overlapping
    // directions, and a differing byte at every position in both orders.
    for length in LENGTHS {
        harness.pair(&left[..length], &right[..length], &left[1..length + 1]);
        harness.pair(&left[..length], &left[..length], &left[..length]);
        harness.pair(&left[..length], &left[1..length + 1], &right[..length]);
        harness.pair(&left[1..length + 1], &left[..length], &right[..length]);
        for position in 0..length {
            right[position] ^= 0x80;
            harness.pair(&left[..length], &right[..length], &left[1..length + 1]);
            harness.pair(&right[..length], &left[..length], &left[1..length + 1]);
            right[position] ^= 0x80;
        }
    }

    let mut state = SEED;
    for index in 0..1024 {
        let length = next_random(&mut state) as usize % (CAPACITY + 1);
        for byte in &mut left {
            *byte = next_random(&mut state) as u8;
        }
        right = left;
        if index % 4 == 1 && length != 0 {
            let position = next_random(&mut state) as usize % length;
            right[position] ^= 0x5a;
        } else if index % 4 >= 2 {
            for byte in &mut right {
                *byte = next_random(&mut state) as u8;
            }
        }
        harness.pair(&left[..length], &right[..length], &left[1..length + 1]);
    }

    for index in 0..512 {
        for byte in &mut left {
            *byte = next_random(&mut state) as u8;
        }
        right = left;
        let position = [0, 2, 6, 15, 31, 32, 1022, 4094][index % 8];
        right[position] ^= 0x80;
        let mut lengths = [0usize; 48];
        for (offset, length) in lengths.iter_mut().enumerate() {
            *length = match offset % 8 {
                0 => 7,
                1 => 0,
                2 => 3,
                3 => 16,
                4 => 32,
                5 => 4095,
                _ => next_random(&mut state) as usize % (CAPACITY + 1),
            };
        }
        if index % 8 == 0 {
            harness.loop_case(&left[..CAPACITY], &left[..CAPACITY], &lengths);
        } else {
            harness.loop_case(&left[..CAPACITY], &right[..CAPACITY], &lengths);
        }
    }

    #[cfg(target_os = "linux")]
    guard_pages::run(&mut harness);
    harness.corpus.flush().expect("flush input transcript");
    println!("layout call constants: seed={SEED:016x}; pair_cases={}; ordinary_pairs={}; guard_page_pairs={}; loop_sequences={}; loop_invocations={}; loop_iterations={}; skipped_guard_invocations={}; checks={}; checksum={:016x}; linux_guard_pages={}",
        harness.pair_cases, harness.pair_cases - harness.guard_page_pairs,
        harness.guard_page_pairs, harness.loop_sequences, 2 * harness.loop_sequences, harness.loop_iterations,
        harness.skipped_guard_calls, harness.checks, harness.checksum, cfg!(target_os = "linux"));
}
