use context_inline::*;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicUsize, Ordering};

const ALIASES: [&str; 40] = [
    "nanos", "nsec", "ns", "usec", "us", "µs", "millis", "msec", "ms", "seconds", "second", "secs",
    "sec", "s", "minutes", "minute", "min", "mins", "m", "hours", "hour", "hr", "hrs", "h", "days",
    "day", "d", "weeks", "week", "wk", "wks", "w", "months", "month", "M", "years", "year", "yr",
    "yrs", "y",
];
const SHORT: [&str; 7] = ["s", "ns", "sec", "nsec", "nanos", "millis", "seconds"];
const SHORT_FUNCTIONS: [fn(&str) -> bool; 7] = [
    short_one,
    short_two,
    short_three,
    short_four,
    short_five,
    short_six,
    short_seven,
];
type Guard = unsafe fn(*const u8, usize) -> u8;
const GUARDS: [Guard; 34] = [
    guard_0, guard_1, guard_2, guard_3, guard_4, guard_5, guard_6, guard_7, guard_8, guard_9,
    guard_10, guard_11, guard_12, guard_13, guard_14, guard_15, guard_16, guard_17, guard_18,
    guard_19, guard_20, guard_21, guard_22, guard_23, guard_24, guard_25, guard_26, guard_27,
    guard_28, guard_29, guard_30, guard_31, guard_32, guard_33,
];
const GUARD_BYTES: &[u8] = b"abcdefghijklmnopqrstuvwxyzabcdefg";
const CAPACITY: usize = 4096;
const SEED: u64 = 0xca71_1fac_e01d_5eed;
static OBSERVER_CALLS: AtomicUsize = AtomicUsize::new(0);

fn next_random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

// Volatile loads prevent the LLVM-built reference from becoming memcmp.
#[inline(never)]
fn byte_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    for index in 0..left.len() {
        // Both slice bounds prove these complete initialized byte reads.
        let a = unsafe { left.as_ptr().add(index).read_volatile() };
        let b = unsafe { right.as_ptr().add(index).read_volatile() };
        if a != b {
            return false;
        }
    }
    true
}

fn chain_reference(input: &[u8]) -> u8 {
    for (index, alias) in ALIASES.iter().enumerate() {
        if byte_equal(input, alias.as_bytes()) {
            return index as u8 + 1;
        }
    }
    0
}

#[inline(never)]
fn byte_hash(input: &[u8]) -> u64 {
    let mut result = input.len() as u64;
    for index in 0..input.len() {
        // Each volatile byte is inside the initialized borrowed range.
        let value = unsafe { input.as_ptr().add(index).read_volatile() };
        result = result.rotate_left(5) ^ u64::from(value).wrapping_mul(0x100_0000_01b3);
    }
    result
}

#[inline(never)]
fn observe(input: &[u8]) -> u64 {
    OBSERVER_CALLS.fetch_add(1, Ordering::Relaxed);
    byte_hash(input)
}

fn ascii(input: &[u8]) -> &str {
    // Every caller constructs letters, ASCII case flips or valid prefixes.
    assert!(input.iter().all(u8::is_ascii));
    unsafe { std::str::from_utf8_unchecked(input) }
}

struct Harness {
    corpus: BufWriter<File>,
    checksum: u64,
    string_cases: usize,
    guard_page_strings: usize,
    loop_sequences: usize,
    loop_invocations: usize,
    loop_iterations: usize,
    fixed_guard_checks: usize,
    skipped_guard_invocations: usize,
    checks: usize,
}

impl Harness {
    fn new(path: &str) -> Self {
        let mut corpus = BufWriter::new(File::create(path).expect("create input transcript"));
        corpus
            .write_all(b"kfrc-context-inline-v1\0")
            .expect("transcript header");
        Self {
            corpus,
            checksum: 0,
            string_cases: 0,
            guard_page_strings: 0,
            loop_sequences: 0,
            loop_invocations: 0,
            loop_iterations: 0,
            fixed_guard_checks: 0,
            skipped_guard_invocations: 0,
            checks: 0,
        }
    }

    fn bytes(&mut self, value: &[u8]) {
        self.corpus
            .write_all(value)
            .expect("write transcript bytes");
    }

    fn number(&mut self, value: usize) {
        self.bytes(&(value as u64).to_le_bytes());
    }

    fn check(&mut self, actual: u64, expected: u64, label: &str, length: usize) {
        assert_eq!(
            actual, expected,
            "{label}; length={length}; string_case={}",
            self.string_cases
        );
        self.checks += 1;
        self.checksum = self.checksum.rotate_left(7)
            ^ actual.wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ length as u64;
    }

    fn string(&mut self, input: &str, other: &str, guarded: bool) {
        self.bytes(&[u8::from(guarded)]);
        self.number(input.len());
        self.number(other.len());
        self.bytes(&[u8::from(input.as_ptr() == other.as_ptr())]);
        self.bytes(input.as_bytes());
        self.bytes(other.as_bytes());
        self.string_cases += 1;
        self.guard_page_strings += usize::from(guarded);
        let length = input.len();
        let expected_chain = chain_reference(input.as_bytes());
        self.check(
            u64::from(literal_chain(input)),
            u64::from(expected_chain),
            "forty literals",
            length,
        );
        for (function, literal) in SHORT_FUNCTIONS.into_iter().zip(SHORT) {
            self.check(
                u64::from(function(input)),
                u64::from(byte_equal(input.as_bytes(), literal.as_bytes())),
                "short literal",
                length,
            );
        }
        let same = byte_equal(input.as_bytes(), other.as_bytes());
        let seconds = byte_equal(input.as_bytes(), b"seconds");
        self.check(
            u64::from(dynamic_rhs(input, other)),
            u64::from(same),
            "dynamic rhs",
            length,
        );
        self.check(
            u64::from(noinline_boundary(input)),
            u64::from(seconds),
            "noinline direct",
            length,
        );
        self.check(
            u64::from(NOINLINE_EQUAL(input, other)),
            u64::from(same),
            "noinline address",
            length,
        );
        self.check(
            u64::from(abi_boundary(input)),
            u64::from(seconds),
            "C ABI direct",
            length,
        );
        // Both str values provide complete readable, initialized UTF-8 ranges;
        // their pointers are non-null even when empty and can legally alias.
        let abi = unsafe { ABI_EQUAL(input.as_ptr(), input.len(), other.as_ptr(), other.len()) };
        self.check(u64::from(abi), u64::from(same), "C ABI address", length);
        let observed = observed_boundary(input, observe);
        self.check(
            u64::from(observed.0),
            u64::from(seconds),
            "observed equality",
            length,
        );
        self.check(
            observed.1,
            byte_hash(input.as_bytes()),
            "complete conversion observation",
            length,
        );
        for choose_first in [false, true] {
            let literal = if choose_first {
                b"seconds".as_slice()
            } else {
                b"day".as_slice()
            };
            let expected = byte_equal(input.as_bytes(), literal);
            self.check(
                u64::from(merged_literal(input, choose_first)),
                u64::from(expected),
                "literal merge",
                length,
            );
        }
        for choose_first in [false, true] {
            let selected = if choose_first { input } else { other };
            let expected = byte_equal(selected.as_bytes(), b"seconds");
            self.check(
                u64::from(merged_dynamic(input, other, "seconds", choose_first)),
                u64::from(expected),
                "dynamic merge",
                length,
            );
        }
    }

    fn guarded_string(&mut self, input: &str, other: &str) {
        self.string(input, other, true);
        for (length, function) in GUARDS.into_iter().enumerate() {
            // This pointer is a full initialized readable string range.
            // Mismatching scalar guards do not access that pointer.
            let actual = unsafe { function(input.as_ptr(), input.len()) };
            let expected = if input.len() == length {
                u64::from(byte_equal(input.as_bytes(), &GUARD_BYTES[..length]))
            } else {
                2
            };
            self.check(
                u64::from(actual),
                expected,
                "fixed length guard",
                input.len(),
            );
            self.fixed_guard_checks += 1;
        }
    }

    fn loops(&mut self, input: &[u8], lengths: &[usize]) {
        assert!(input.is_ascii());
        assert!(lengths.iter().all(|&length| length <= input.len()));
        self.bytes(&[2]);
        self.number(input.len());
        self.bytes(input);
        self.number(lengths.len());
        for &length in lengths {
            self.number(length);
        }
        self.loop_sequences += 1;
        self.loop_invocations += 2;
        self.loop_iterations += 2 * lengths.len();
        let mut changing = 0u64;
        let mut refreshed = 0u64;
        for &length in lengths {
            let bytes = &input[..length];
            let first = byte_equal(bytes, b"seconds");
            let second = byte_equal(bytes, b"day");
            changing = changing.rotate_left(5)
                ^ (length as u64).wrapping_mul(17)
                ^ (u64::from(first) | (u64::from(second) << 1));
            let value = if length == 7 { u64::from(first) } else { 2 };
            refreshed = refreshed.rotate_left(3) ^ (length as u64).wrapping_mul(13) ^ value;
        }
        // Every complete prefix is initialized/readable and ASCII. The input
        // remains immutable throughout both target loops and their references.
        let actual = unsafe { varying_prefixes(input.as_ptr(), lengths) };
        self.check(actual, changing, "varying length loop", lengths.len());
        let actual = unsafe { refreshed_guards(input.as_ptr(), lengths) };
        self.check(actual, refreshed, "refreshed length loop", lengths.len());
    }

    /// No function call below may read `unreadable`: every length mismatches.
    unsafe fn skipped_guards(&mut self, unreadable: *const u8) {
        for (guard_length, function) in GUARDS.into_iter().enumerate() {
            for length in [guard_length + 1, usize::MAX] {
                self.bytes(&[3]);
                self.number(guard_length);
                self.number(length);
                let actual = unsafe { function(unreadable, length) };
                self.check(u64::from(actual), 2, "unreadable unmatched guard", length);
                self.skipped_guard_invocations += 1;
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod guard_pages {
    use super::{Harness, GUARD_BYTES};
    use std::ffi::{c_int, c_void};
    use std::ptr::NonNull;

    const PROT_NONE: c_int = 0;
    const PROT_READ: c_int = 1;
    const PROT_WRITE: c_int = 2;
    const MAP_PRIVATE: c_int = 2;
    const MAP_ANONYMOUS: c_int = 0x20;

    extern "C" {
        fn getpagesize() -> c_int;
        fn mmap(
            address: *mut c_void,
            length: usize,
            protection: c_int,
            flags: c_int,
            fd: c_int,
            offset: isize,
        ) -> *mut c_void;
        fn mprotect(address: *mut c_void, length: usize, protection: c_int) -> c_int;
        fn munmap(address: *mut c_void, length: usize) -> c_int;
    }

    struct Pages {
        base: *mut u8,
        page_size: usize,
    }

    impl Pages {
        fn new() -> Self {
            let page_size = unsafe { getpagesize() };
            assert!(page_size > 0);
            let page_size = page_size as usize;
            // mmap returns a fresh two-page private mapping owned by Pages.
            let address = unsafe {
                mmap(
                    std::ptr::null_mut(),
                    page_size * 2,
                    PROT_READ | PROT_WRITE,
                    MAP_PRIVATE | MAP_ANONYMOUS,
                    -1,
                    0,
                )
            };
            assert_ne!(
                address as isize,
                -1,
                "mmap: {}",
                std::io::Error::last_os_error()
            );
            assert!(!address.is_null());
            let pages = Self {
                base: address.cast(),
                page_size,
            };
            let status =
                unsafe { mprotect(pages.base.add(page_size).cast(), page_size, PROT_NONE) };
            assert_eq!(status, 0, "mprotect: {}", std::io::Error::last_os_error());
            pages
        }

        fn tail(&mut self, bytes: &[u8]) -> *const u8 {
            assert!(bytes.len() <= self.page_size);
            assert!(bytes.is_ascii());
            if bytes.is_empty() {
                return NonNull::<u8>::dangling().as_ptr().cast_const();
            }
            // The entire initialized ASCII string ends exactly where the next
            // page becomes PROT_NONE. No byte reference crosses that boundary.
            let start = unsafe { self.base.add(self.page_size - bytes.len()) };
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), start, bytes.len()) };
            start.cast_const()
        }

        fn unreadable(&self) -> *const u8 {
            unsafe { self.base.add(self.page_size).cast_const() }
        }
    }

    impl Drop for Pages {
        fn drop(&mut self) {
            // No mapped reference or pointer is used after this unique owner.
            let status = unsafe { munmap(self.base.cast(), self.page_size * 2) };
            assert_eq!(status, 0, "munmap: {}", std::io::Error::last_os_error());
        }
    }

    /// Both complete ranges are initialized ASCII and mappings remain live.
    unsafe fn run_case(harness: &mut Harness, left: *const u8, right: *const u8, length: usize) {
        let left = unsafe { std::slice::from_raw_parts(left, length) };
        let right = unsafe { std::slice::from_raw_parts(right, length) };
        let left = unsafe { std::str::from_utf8_unchecked(left) };
        let right = unsafe { std::str::from_utf8_unchecked(right) };
        harness.guarded_string(left, right);
    }

    pub fn run(harness: &mut Harness) {
        let mut left_pages = Pages::new();
        let mut right_pages = Pages::new();
        for length in 0..=33 {
            let right = &GUARD_BYTES[..length];
            let mut left = right.to_vec();
            let left_pointer = left_pages.tail(&left);
            let right_pointer = right_pages.tail(right);
            unsafe {
                run_case(harness, left_pointer, right_pointer, length);
                run_case(harness, left_pointer, left_pointer, length);
            }
            for position in 0..length {
                left[position] ^= 0x20;
                let left_pointer = left_pages.tail(&left);
                unsafe {
                    run_case(harness, left_pointer, right_pointer, length);
                    run_case(harness, right_pointer, left_pointer, length);
                }
                left[position] ^= 0x20;
            }
        }
        // Only raw pointers reach the false-guard tests; no &str or slice is
        // ever created for protected or usize::MAX-byte memory.
        unsafe { harness.skipped_guards(left_pages.unreadable()) };
    }
}

fn main() {
    let transcript = std::env::args()
        .nth(1)
        .expect("input transcript output path");
    let mut harness = Harness::new(&transcript);
    for alias in ALIASES {
        let owned = alias.to_owned();
        harness.string(alias, alias, false);
        harness.string(&owned, alias, false);
        harness.string(&owned, &owned, false);
        for position in 0..alias.len() {
            let mut changed = alias.as_bytes().to_vec();
            // Replacing either byte of µ separately could make invalid UTF-8.
            // Its two positions get distinct ASCII replacements of that whole
            // scalar, retaining one valid input for every original byte index.
            if changed[position].is_ascii() {
                changed[position] ^= 0x20;
            } else {
                changed[0] = b'x';
                changed[1] = if position == 0 { b'0' } else { b'1' };
            }
            let changed = std::str::from_utf8(&changed).expect("valid alias mutation");
            harness.string(changed, alias, false);
            harness.string(alias, changed, false);
        }
    }

    let mut left = [0u8; CAPACITY + 1];
    for (index, byte) in left.iter_mut().enumerate() {
        *byte = b'a' + (index % 26) as u8;
    }
    let mut right = left;
    for length in 0..=65 {
        harness.string(ascii(&left[..length]), ascii(&right[..length]), false);
        harness.string(ascii(&left[..length]), ascii(&left[..length]), false);
        harness.string(ascii(&left[..length]), ascii(&left[1..length + 1]), false);
        harness.string(ascii(&left[1..length + 1]), ascii(&left[..length]), false);
        harness.string(ascii(&left[..length]), ascii(&right[..length + 1]), false);
        for position in 0..length {
            right[position] ^= 0x20;
            harness.string(ascii(&left[..length]), ascii(&right[..length]), false);
            harness.string(ascii(&right[..length]), ascii(&left[..length]), false);
            right[position] ^= 0x20;
        }
    }

    let mut state = SEED;
    for index in 0..512 {
        let length = next_random(&mut state) as usize % (CAPACITY + 1);
        let other_length = if index % 4 == 0 {
            next_random(&mut state) as usize % (CAPACITY + 1)
        } else {
            length
        };
        for byte in &mut left {
            *byte = b'a' + (next_random(&mut state) % 26) as u8;
        }
        right = left;
        if index % 4 == 1 && length != 0 {
            right[next_random(&mut state) as usize % length] ^= 0x20;
        } else if index % 4 >= 2 {
            for byte in &mut right {
                *byte = b'a' + (next_random(&mut state) % 26) as u8;
            }
        }
        harness.string(ascii(&left[..length]), ascii(&right[..other_length]), false);
    }
    for index in 0..32 {
        harness.string(ALIASES[index], ALIASES[(index + 17) % ALIASES.len()], false);
    }

    for index in 0..256 {
        for byte in &mut left {
            *byte = b'a' + (next_random(&mut state) % 26) as u8;
        }
        if index % 3 == 0 {
            left[..7].copy_from_slice(b"seconds");
        }
        if index % 3 == 1 {
            left[..3].copy_from_slice(b"day");
        }
        let mut lengths = [0usize; 48];
        for (offset, length) in lengths.iter_mut().enumerate() {
            *length = match offset % 8 {
                0 => 7,
                1 => 0,
                2 => 3,
                3 => 1,
                4 => 32,
                5 => 4095,
                _ => next_random(&mut state) as usize % (CAPACITY + 1),
            };
        }
        harness.loops(&left[..CAPACITY], &lengths);
    }
    #[cfg(target_os = "linux")]
    guard_pages::run(&mut harness);
    harness.corpus.flush().expect("flush input transcript");
    let observer_calls = OBSERVER_CALLS.load(Ordering::Relaxed);
    assert_eq!(
        observer_calls, harness.string_cases,
        "one complete observer per string"
    );
    println!("context inline: seed={SEED:016x}; literal_sites=40; string_cases={}; ordinary_strings={}; guard_page_strings={}; loop_sequences={}; loop_invocations={}; loop_iterations={}; fixed_guard_checks={}; skipped_guard_invocations={}; observer_calls={observer_calls}; checks={}; checksum={:016x}; linux_guard_pages={}",
        harness.string_cases, harness.string_cases - harness.guard_page_strings,
        harness.guard_page_strings, harness.loop_sequences, harness.loop_invocations,
        harness.loop_iterations, harness.fixed_guard_checks, harness.skipped_guard_invocations,
        harness.checks, harness.checksum, cfg!(target_os = "linux"));
}
