#![crate_type = "rlib"]

// Keep forty distinct string-equality sites in one body. With MIR0/sharedstd,
// the str -> bytes adapters and slice equality remain visible to codegen.
// The ordinal makes every successful literal observable, including aliases
// after the existing thirty-two costed-call limit.
#[inline(never)]
pub fn literal_chain(input: &str) -> u8 {
    if input == "nanos" {
        return 1;
    }
    if input == "nsec" {
        return 2;
    }
    if input == "ns" {
        return 3;
    }
    if input == "usec" {
        return 4;
    }
    if input == "us" {
        return 5;
    }
    if input == "µs" {
        return 6;
    }
    if input == "millis" {
        return 7;
    }
    if input == "msec" {
        return 8;
    }
    if input == "ms" {
        return 9;
    }
    if input == "seconds" {
        return 10;
    }
    if input == "second" {
        return 11;
    }
    if input == "secs" {
        return 12;
    }
    if input == "sec" {
        return 13;
    }
    if input == "s" {
        return 14;
    }
    if input == "minutes" {
        return 15;
    }
    if input == "minute" {
        return 16;
    }
    if input == "min" {
        return 17;
    }
    if input == "mins" {
        return 18;
    }
    if input == "m" {
        return 19;
    }
    if input == "hours" {
        return 20;
    }
    if input == "hour" {
        return 21;
    }
    if input == "hr" {
        return 22;
    }
    if input == "hrs" {
        return 23;
    }
    if input == "h" {
        return 24;
    }
    if input == "days" {
        return 25;
    }
    if input == "day" {
        return 26;
    }
    if input == "d" {
        return 27;
    }
    if input == "weeks" {
        return 28;
    }
    if input == "week" {
        return 29;
    }
    if input == "wk" {
        return 30;
    }
    if input == "wks" {
        return 31;
    }
    if input == "w" {
        return 32;
    }
    if input == "months" {
        return 33;
    }
    if input == "month" {
        return 34;
    }
    if input == "M" {
        return 35;
    }
    if input == "years" {
        return 36;
    }
    if input == "year" {
        return 37;
    }
    if input == "yr" {
        return 38;
    }
    if input == "yrs" {
        return 39;
    }
    if input == "y" {
        return 40;
    }
    0
}

macro_rules! short_equalities {
    ($($name:ident => $literal:literal),+ $(,)?) => {
        $(
            #[inline(never)]
            pub fn $name(input: &str) -> bool {
                // Use the same fixed str-to-bytes conversion as str equality.
                input.as_bytes() == $literal.as_bytes()
            }
        )+
    };
}

short_equalities! {
    short_one => "s",
    short_two => "ns",
    short_three => "sec",
    short_four => "nsec",
    short_five => "nanos",
    short_six => "millis",
    short_seven => "seconds",
}

/// Both lengths are runtime values, so this is the no-literal-fact control.
#[inline(never)]
pub fn dynamic_rhs(left: &str, right: &str) -> bool {
    left.as_bytes() == right.as_bytes()
}

#[inline(never)]
fn noinline_equal(left: &str, right: &str) -> bool {
    left.as_bytes() == right.as_bytes()
}

// Taking the address also preserves a real callable noinline boundary.
pub static NOINLINE_EQUAL: fn(&str, &str) -> bool = noinline_equal;

#[inline(never)]
pub fn noinline_boundary(input: &str) -> bool {
    noinline_equal(input, "seconds")
}

/// # Safety
/// Each pointer is non-null and readable for its whole initialized length,
/// and the bytes are valid UTF-8. The ranges may alias or overlap. All ranges
/// and the two strings created from them stay live throughout the call.
#[inline(never)]
pub unsafe extern "C" fn abi_equal(
    left: *const u8,
    left_length: usize,
    right: *const u8,
    right_length: usize,
) -> bool {
    // The caller provides the full readable UTF-8 ranges, including the
    // non-null aligned pointer requirement when the length is zero.
    let left = unsafe { std::slice::from_raw_parts(left, left_length) };
    let right = unsafe { std::slice::from_raw_parts(right, right_length) };
    let left = unsafe { std::str::from_utf8_unchecked(left) };
    let right = unsafe { std::str::from_utf8_unchecked(right) };
    left.as_bytes() == right.as_bytes()
}

pub static ABI_EQUAL: unsafe extern "C" fn(*const u8, usize, *const u8, usize) -> bool = abi_equal;

#[inline(never)]
pub fn abi_boundary(input: &str) -> bool {
    let literal = "seconds";
    // A str's bytes supply the complete initialized UTF-8 range. Neither
    // borrow is mutated or outlived by the noinline C-ABI call.
    unsafe { abi_equal(input.as_ptr(), input.len(), literal.as_ptr(), literal.len()) }
}

/// Conversion and an external observer execute before the comparison. The
/// callback can observe defined bytes; it cannot manufacture invalid values.
#[inline(never)]
pub fn observed_boundary(input: &str, observer: fn(&[u8]) -> u64) -> (bool, u64) {
    let bytes = input.as_bytes();
    let observed = observer(bytes);
    (bytes == "seconds".as_bytes(), observed)
}

#[inline(never)]
pub fn merged_literal(input: &str, choose_first: bool) -> bool {
    let literal = if choose_first { "seconds" } else { "day" };
    input.as_bytes() == literal.as_bytes()
}

#[inline(never)]
pub fn merged_dynamic(first: &str, second: &str, expected: &str, choose_first: bool) -> bool {
    let selected = if choose_first {
        first.as_bytes()
    } else {
        second.as_bytes()
    };
    selected == expected.as_bytes()
}

macro_rules! guarded_literals {
    ($($name:ident => $literal:literal),+ $(,)?) => {
        $(
            /// # Safety
            /// When the scalar guard matches, `pointer` is non-null and
            /// readable for its complete initialized `length` byte range.
            /// Unmatched guards access nothing. A zero-length match still
            /// requires a non-null aligned pointer.
            #[inline(never)]
            pub unsafe fn $name(pointer: *const u8, length: usize) -> u8 {
                if length != $literal.len() {
                    return 2;
                }
                // Only this guarded, fully initialized range is observed.
                let bytes = unsafe { std::slice::from_raw_parts(pointer, length) };
                u8::from(bytes == $literal.as_bytes())
            }
        )+
    };
}

guarded_literals! {
    guard_0 => "",
    guard_1 => "a",
    guard_2 => "ab",
    guard_3 => "abc",
    guard_4 => "abcd",
    guard_5 => "abcde",
    guard_6 => "abcdef",
    guard_7 => "abcdefg",
    guard_8 => "abcdefgh",
    guard_9 => "abcdefghi",
    guard_10 => "abcdefghij",
    guard_11 => "abcdefghijk",
    guard_12 => "abcdefghijkl",
    guard_13 => "abcdefghijklm",
    guard_14 => "abcdefghijklmn",
    guard_15 => "abcdefghijklmno",
    guard_16 => "abcdefghijklmnop",
    guard_17 => "abcdefghijklmnopq",
    guard_18 => "abcdefghijklmnopqr",
    guard_19 => "abcdefghijklmnopqrs",
    guard_20 => "abcdefghijklmnopqrst",
    guard_21 => "abcdefghijklmnopqrstu",
    guard_22 => "abcdefghijklmnopqrstuv",
    guard_23 => "abcdefghijklmnopqrstuvw",
    guard_24 => "abcdefghijklmnopqrstuvwx",
    guard_25 => "abcdefghijklmnopqrstuvwxy",
    guard_26 => "abcdefghijklmnopqrstuvwxyz",
    guard_27 => "abcdefghijklmnopqrstuvwxyza",
    guard_28 => "abcdefghijklmnopqrstuvwxyzab",
    guard_29 => "abcdefghijklmnopqrstuvwxyzabc",
    guard_30 => "abcdefghijklmnopqrstuvwxyzabcd",
    guard_31 => "abcdefghijklmnopqrstuvwxyzabcde",
    guard_32 => "abcdefghijklmnopqrstuvwxyzabcdef",
    guard_33 => "abcdefghijklmnopqrstuvwxyzabcdefg",
}

/// # Safety
/// `pointer` is non-null and initialized/readable for at least the largest
/// element of `lengths`. The sequences used here have only ASCII bytes, so
/// every prefix of these lengths is valid UTF-8. Ranges stay live and are not
/// mutated by the caller while either loop is executing.
#[inline(never)]
pub unsafe fn varying_prefixes(pointer: *const u8, lengths: &[usize]) -> u64 {
    let mut result = 0u64;
    for &length in lengths {
        let bytes = unsafe { std::slice::from_raw_parts(pointer, length) };
        let input = unsafe { std::str::from_utf8_unchecked(bytes) };
        let first = input.as_bytes() == "seconds".as_bytes();
        let second = input.as_bytes() == "day".as_bytes();
        let value = u64::from(first) | (u64::from(second) << 1);
        result = result.rotate_left(5) ^ (length as u64).wrapping_mul(17) ^ value;
    }
    result
}

/// # Safety
/// The same full readable ASCII-prefix contract as `varying_prefixes` applies.
#[inline(never)]
pub unsafe fn refreshed_guards(pointer: *const u8, lengths: &[usize]) -> u64 {
    let mut result = 0u64;
    for &length in lengths {
        let value = if length == 7 {
            let bytes = unsafe { std::slice::from_raw_parts(pointer, length) };
            u64::from(bytes == "seconds".as_bytes())
        } else {
            2
        };
        result = result.rotate_left(3) ^ (length as u64).wrapping_mul(13) ^ value;
    }
    result
}
