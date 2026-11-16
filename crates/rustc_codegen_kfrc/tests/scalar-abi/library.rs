//! Compile this entire crate with the selected backend. Calls into `foreign`
//! exercise the other direction of the boundary from the LLVM driver.
use scalar_abi_foreign as foreign;

macro_rules! scalar {
    ($module:ident, $ty:ty, $value:ident => $body:expr,
     $check:ident, $dirty_c:ident, $dirty_rust:ident) => {
        pub mod $module {
            use super::foreign;
            #[inline(never)]
            pub fn rust($value: $ty) -> $ty {
                $body
            }
            #[inline(never)]
            pub extern "C" fn c($value: $ty) -> $ty {
                $body
            }
            #[inline(never)]
            pub fn echo_rust(value: $ty) -> u64 {
                value as i64 as u64
            }
            #[inline(never)]
            pub extern "C" fn echo_c(value: $ty) -> u64 {
                value as i64 as u64
            }
            #[inline(never)]
            pub fn direct_rust(value: $ty) -> u64 {
                foreign::$module::rust(value) as i64 as u64
            }
            #[inline(never)]
            pub fn direct_c(value: $ty) -> u64 {
                foreign::$module::c(value) as i64 as u64
            }
            #[inline(never)]
            pub fn indirect_rust(function: fn($ty) -> $ty, value: $ty) -> u64 {
                function(value) as i64 as u64
            }
            #[inline(never)]
            pub fn indirect_c(function: extern "C" fn($ty) -> $ty, value: $ty) -> u64 {
                function(value) as i64 as u64
            }
            #[inline(never)]
            pub fn checked_argument(value: $ty) -> u64 {
                unsafe { foreign::$module::$check(value) }
            }
            #[inline(never)]
            pub fn dirty_c_result(value: u64) -> u64 {
                unsafe { foreign::$module::$dirty_c(value) as i64 as u64 }
            }
            #[inline(never)]
            pub fn dirty_rust_result(value: u64) -> u64 {
                unsafe { foreign::$module::$dirty_rust(value) as i64 as u64 }
            }
        }
    };
}

scalar!(boolean, bool, x => !x, check_c_boolean, dirty_c_boolean, dirty_rust_boolean);
scalar!(unsigned8, u8, x => x.wrapping_mul(37).wrapping_add(11),
    check_c_unsigned8, dirty_c_unsigned8, dirty_rust_unsigned8);
scalar!(signed8, i8, x => x.wrapping_mul(37).wrapping_add(11),
    check_c_signed8, dirty_c_signed8, dirty_rust_signed8);
scalar!(unsigned16, u16, x => x.wrapping_mul(37).wrapping_add(11),
    check_c_unsigned16, dirty_c_unsigned16, dirty_rust_unsigned16);
scalar!(signed16, i16, x => x.wrapping_mul(37).wrapping_add(11),
    check_c_signed16, dirty_c_signed16, dirty_rust_signed16);
scalar!(unsigned32, u32, x => x.wrapping_mul(37).wrapping_add(11),
    check_c_unsigned32, dirty_c_unsigned32, dirty_rust_unsigned32);
scalar!(signed32, i32, x => x.wrapping_mul(37).wrapping_add(11),
    check_c_signed32, dirty_c_signed32, dirty_rust_signed32);

#[inline(never)]
pub fn pair(value: foreign::Pair, bias: i16) -> foreign::Pair {
    foreign::Pair {
        number: value.number.wrapping_add(bias),
        predicate: !value.predicate,
    }
}

#[inline(never)]
pub fn direct_pair(value: foreign::Pair, bias: i16) -> foreign::Pair {
    foreign::pair(value, bias)
}

#[inline(never)]
pub fn indirect_pair(
    function: fn(foreign::Pair, i16) -> foreign::Pair,
    value: foreign::Pair,
    bias: i16,
) -> foreign::Pair {
    function(value, bias)
}

#[inline(never)]
pub fn dirty_pair(number: u64, predicate: bool) -> u64 {
    let value = unsafe { foreign::dirty_pair(number, predicate) };
    (value.number as i64 as u64).wrapping_add(value.predicate as u64)
}

#[inline(never)]
pub fn checked_pair(value: foreign::Pair) -> u64 {
    unsafe { foreign::check_pair(value) }
}

#[inline(never)]
pub fn wrapper(value: foreign::SignedByte) -> foreign::SignedByte {
    foreign::SignedByte(value.0.wrapping_add(11))
}

#[inline(never)]
pub extern "C" fn c_wrapper(value: foreign::SignedByte) -> foreign::SignedByte {
    foreign::wrapper(value)
}

#[inline(never)]
pub fn checked_wrapper(value: foreign::SignedByte) -> u64 {
    unsafe { foreign::check_signed_wrapper(value) }
}

#[inline(never)]
pub fn stack_rust(a: u64, b: u64, c: u64, d: u64, e: u64, f: u64, number: i16, flag: bool) -> u64 {
    (a ^ b ^ c ^ d ^ e ^ f ^ (number as i64 as u64)).wrapping_add(flag as u64)
}

#[inline(never)]
pub extern "C" fn stack_c(
    a: u64,
    b: u64,
    c: u64,
    d: u64,
    e: u64,
    f: u64,
    number: i16,
    flag: bool,
) -> u64 {
    (a ^ b ^ c ^ d ^ e ^ f ^ (number as i64 as u64)).wrapping_add(flag as u64)
}

#[inline(never)]
pub fn write_byte(bytes: &mut [u8; 3], value: u8) -> u8 {
    bytes[1] = value;
    value
}
