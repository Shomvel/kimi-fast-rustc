#![feature(core_intrinsics)]
#![allow(internal_features)]

#[inline(never)]
unsafe fn through_hint(value: u64) -> u64 {
    unsafe { std::hint::assert_unchecked(value > 0) };
    value.wrapping_add(3)
}

#[inline(never)]
unsafe fn through_intrinsic(value: u64) -> u64 {
    unsafe { std::intrinsics::assume(value < 100) };
    value.wrapping_mul(2)
}

#[inline(never)]
unsafe fn generic_assume<T: Copy>(condition: bool, value: T) -> T {
    unsafe { std::intrinsics::assume(condition) };
    value
}

fn main() {
    // Execute only inputs satisfying each unsafe contract. False/poison cases
    // belong to source-IR diagnostics, not native UB execution comparisons.
    for value in [1, 7, 99] {
        let hint = unsafe { through_hint(value) };
        let intrinsic = unsafe { through_intrinsic(value) };
        println!("{value} {hint} {intrinsic}");
    }
    let unsigned = unsafe { generic_assume(true, 17u32) };
    let signed = unsafe { generic_assume(true, -23i64) };
    println!("generic {unsigned} {signed}");
}
