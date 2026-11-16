//! LLVM-compiled callees, shared types and assembly boundary declarations.
#![allow(improper_ctypes)]

macro_rules! scalar {
    ($module:ident, $ty:ty, $value:ident => $body:expr,
     $check:ident, $dirty_c:ident, $dirty_rust:ident, $poison_c:ident, $poison_rust:ident) => {
        pub mod $module {
            #[inline(never)]
            pub fn rust($value: $ty) -> $ty {
                $body
            }
            #[inline(never)]
            pub extern "C" fn c($value: $ty) -> $ty {
                $body
            }
            unsafe extern "C" {
                pub fn $check(value: $ty) -> u64;
                pub fn $dirty_c(value: u64) -> $ty;
                pub fn $poison_c(function: extern "C" fn($ty) -> u64, value: u64) -> u64;
                pub fn $poison_rust(function: fn($ty) -> u64, value: u64) -> u64;
            }
            unsafe extern "Rust" {
                pub fn $dirty_rust(value: u64) -> $ty;
            }
        }
    };
}

scalar!(boolean, bool, x => !x,
    check_c_boolean, dirty_c_boolean, dirty_rust_boolean, poison_c_boolean, poison_rust_boolean);
scalar!(unsigned8, u8, x => x.wrapping_mul(37).wrapping_add(11),
    check_c_unsigned8, dirty_c_unsigned8, dirty_rust_unsigned8, poison_c_unsigned8, poison_rust_unsigned8);
scalar!(signed8, i8, x => x.wrapping_mul(37).wrapping_add(11),
    check_c_signed8, dirty_c_signed8, dirty_rust_signed8, poison_c_signed8, poison_rust_signed8);
scalar!(unsigned16, u16, x => x.wrapping_mul(37).wrapping_add(11),
    check_c_unsigned16, dirty_c_unsigned16, dirty_rust_unsigned16, poison_c_unsigned16, poison_rust_unsigned16);
scalar!(signed16, i16, x => x.wrapping_mul(37).wrapping_add(11),
    check_c_signed16, dirty_c_signed16, dirty_rust_signed16, poison_c_signed16, poison_rust_signed16);
scalar!(unsigned32, u32, x => x.wrapping_mul(37).wrapping_add(11),
    check_c_unsigned32, dirty_c_unsigned32, dirty_rust_unsigned32, poison_c_unsigned32, poison_rust_unsigned32);
scalar!(signed32, i32, x => x.wrapping_mul(37).wrapping_add(11),
    check_c_signed32, dirty_c_signed32, dirty_rust_signed32, poison_c_signed32, poison_rust_signed32);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Pair {
    pub number: i16,
    pub predicate: bool,
}

#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct SignedByte(pub i8);

#[inline(never)]
pub fn pair(value: Pair, bias: i16) -> Pair {
    Pair {
        number: value.number.wrapping_add(bias),
        predicate: !value.predicate,
    }
}

#[inline(never)]
pub extern "C" fn wrapper(value: SignedByte) -> SignedByte {
    SignedByte(value.0.wrapping_add(11))
}

unsafe extern "Rust" {
    pub fn dirty_pair(number: u64, predicate: bool) -> Pair;
    pub fn check_pair(value: Pair) -> u64;
}

unsafe extern "C" {
    // The representation is one signed byte, but it is not a primitive Rust
    // integer type. The producer's signext attribute must survive that fact.
    pub fn check_signed_wrapper(value: SignedByte) -> u64;
}

pub type RustStack = fn(u64, u64, u64, u64, u64, u64, i16, bool) -> u64;
pub type CStack = extern "C" fn(u64, u64, u64, u64, u64, u64, i16, bool) -> u64;

unsafe extern "C" {
    pub fn poison_stack_rust(function: RustStack, number: u64, predicate: u64) -> u64;
    pub fn poison_stack_c(function: CStack, number: u64, predicate: u64) -> u64;
}
