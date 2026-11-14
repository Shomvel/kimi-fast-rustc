#![crate_type = "rlib"]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Pair {
    pub value: u64,
    pub tag: u64,
}

pub const TAG: u64 = 0x715c_039a_e42b_806d;
pub const SALT: u64 = 0x3a97_f01d_5b62_c8e4;

unsafe extern "Rust" {
    fn cgscc_observe(value: u64) -> u64;
}

unsafe extern "C" {
    fn cgscc_external_pair(value: Pair) -> Pair;
}

// The straight-line failure path keeps each unprepared wrapper above the old
// 128-node scratch limit. It is observable when used by dynamic_guard below.
// A leaf's constant tag must cross the real call before this path can vanish.
macro_rules! checked_pair {
    ($value:expr) => {{
        let pair = $value;
        if pair.tag == TAG {
            pair
        } else {
            let mut value = pair.value;
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            value = unsafe { cgscc_observe(value) };
            Pair { value, tag: 0 }
        }
    }};
}

#[inline]
fn leaf(value: u64) -> Pair {
    Pair {
        value: value ^ SALT,
        tag: TAG,
    }
}

macro_rules! wrapper {
    ($name:ident, $child:ident) => {
        #[inline]
        fn $name(value: u64) -> Pair {
            checked_pair!($child(value))
        }
    };
}

// Seven levels exceed both the old two-wave scratch policy and one shallow
// caller walk. No always-inline annotation grants a size or depth waiver.
wrapper!(stage_7, leaf);
wrapper!(stage_6, stage_7);
wrapper!(stage_5, stage_6);
wrapper!(stage_4, stage_5);
wrapper!(stage_3, stage_4);
wrapper!(stage_2, stage_3);
wrapper!(stage_1, stage_2);

#[unsafe(no_mangle)]
#[inline(never)]
pub fn cgscc_chain(value: u64) -> Pair {
    stage_1(value)
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn cgscc_diamond(value: u64) -> u64 {
    let left = stage_1(value);
    let right = stage_1(!value);
    left.value ^ right.value.rotate_left(7) ^ left.tag ^ right.tag
}

// Exercise the same failure code with a dynamic tag; it must retain all 32
// observations and their order, even though the positive chain loses them.
#[unsafe(no_mangle)]
#[inline(never)]
pub fn cgscc_dynamic_guard(value: Pair) -> Pair {
    checked_pair!(value)
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn cgscc_noinline(value: u64) -> u64 {
    value.rotate_left(9) ^ SALT
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn cgscc_noinline_caller(value: u64) -> u64 {
    cgscc_noinline(value) ^ value
}

// An address-bearing exported C ABI stays callable from the independent
// LLVM driver, directly and through its original function pointer.
#[unsafe(no_mangle)]
#[inline(never)]
pub extern "C" fn cgscc_pair_abi(value: Pair) -> Pair {
    Pair {
        value: value.value ^ value.tag,
        tag: value.tag.rotate_left(3),
    }
}

pub static PAIR_READER: extern "C" fn(Pair) -> Pair = cgscc_pair_abi;

#[unsafe(no_mangle)]
#[inline(never)]
pub fn cgscc_external_caller(value: Pair) -> Pair {
    unsafe { cgscc_external_pair(value) }
}

#[inline]
fn recursive_a(depth: u64, value: u64) -> u64 {
    if depth == 0 {
        value
    } else {
        recursive_b(depth - 1, value ^ depth)
    }
}

#[inline]
fn recursive_b(depth: u64, value: u64) -> u64 {
    if depth == 0 {
        !value
    } else {
        recursive_a(depth - 1, value.rotate_left(3))
    }
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn cgscc_recursive(depth: u64, value: u64) -> u64 {
    recursive_a(depth, value)
}
