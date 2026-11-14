#![crate_type = "rlib"]

#[inline(never)]
fn pure_mix(value: u64) -> u64 {
    value.wrapping_mul(0x9e37_79b9).rotate_left(17) ^ value.rotate_right(11)
}

#[inline(never)]
unsafe fn read_raw(pointer: *const u64) -> u64 {
    unsafe { *pointer }
}

// Taking its address keeps this direct callee's original pointer ABI. The
// driver also calls it through the exported static after all other probes.
pub static RAW_READER: unsafe fn(*const u64) -> u64 = read_raw;

#[inline(never)]
unsafe fn write_raw(pointer: *mut u64, value: u64) -> u64 {
    unsafe {
        let before = *pointer;
        *pointer = value;
        before
    }
}

unsafe extern "Rust" {
    fn ipo_external_write(pointer: *mut u64, value: u64) -> u64;
}

/// # Safety
/// `pointer` is initialized, aligned and writable for the whole call.
#[inline(never)]
pub unsafe fn readnone_reload(pointer: *mut u64, value: u64) -> (u64, u64) {
    unsafe {
        *pointer = value;
        let mixed = pure_mix(value);
        (*pointer, mixed)
    }
}

/// # Safety
/// `pointer` is initialized, aligned and writable for the whole call.
#[inline(never)]
pub unsafe fn readonly_reload(pointer: *mut u64, value: u64) -> (u64, u64) {
    unsafe {
        *pointer = value;
        let observed = read_raw(pointer);
        (observed, *pointer)
    }
}

/// # Safety
/// `pointer` is initialized, aligned and writable for the whole call.
#[inline(never)]
pub unsafe fn writing_reload(pointer: *mut u64, value: u64, next: u64) -> (u64, u64) {
    unsafe {
        *pointer = value;
        let observed = write_raw(pointer, next);
        (observed, *pointer)
    }
}

/// # Safety
/// `pointer` is initialized, aligned and writable for the whole call.
#[inline(never)]
pub unsafe fn unknown_reload(pointer: *mut u64, value: u64, next: u64) -> (u64, u64) {
    unsafe {
        *pointer = value;
        let observed = ipo_external_write(pointer, next);
        (observed, *pointer)
    }
}

/// # Safety
/// `pointer` is initialized, aligned and writable for the whole call, and the
/// callback accepts it under that contract.
#[inline(never)]
pub unsafe fn indirect_reload(
    pointer: *mut u64,
    value: u64,
    next: u64,
    callback: unsafe fn(*mut u64, u64) -> u64,
) -> (u64, u64) {
    unsafe {
        *pointer = value;
        let observed = callback(pointer, next);
        (observed, *pointer)
    }
}

#[inline(never)]
fn scalar_read(value: &u64) -> u64 {
    *value
}

#[inline(never)]
fn sum_fields(value: &(u64, u64)) -> u64 {
    value.0.wrapping_add(value.1)
}

#[inline(never)]
pub fn promoted_reads(a: u64, b: u64) -> u64 {
    let values = (a, b);
    scalar_read(&values.0) ^ sum_fields(&values)
}

#[inline(never)]
fn xor_fields_two(value: &(u64, u64)) -> u64 {
    value.0 ^ value.1
}

#[inline(never)]
fn xor_fields_three(value: &(u64, u64, u64)) -> u64 {
    value.0 ^ value.1 ^ value.2
}

// Primitive XOR keeps both callees free of helper calls before promotion.
// sum_fields above continues to exercise the conservative call barrier.
#[inline(never)]
pub fn promoted_field_reads(a: u64, b: u64, c: u64) -> (u64, u64) {
    let pair = (a, b);
    let triple = (a, b, c);
    (xor_fields_two(&pair), xor_fields_three(&triple))
}

#[inline(never)]
fn addressed_read(value: &u64) -> u64 {
    *value
}

pub static REFERENCE_READER: fn(&u64) -> u64 = addressed_read;

#[inline(never)]
pub fn addressed_direct(value: &u64) -> u64 {
    addressed_read(value)
}

/// # Safety
/// `output` is writable for three bytes and does not alias this frame.
#[inline(never)]
pub unsafe fn private_copy_three(value: u64, output: *mut u8) {
    let local = value;
    unsafe {
        core::ptr::copy_nonoverlapping((&raw const local).cast::<u8>().add(2), output, 3);
    }
}

/// # Safety
/// `output` is writable for six bytes and does not alias this frame.
#[inline(never)]
pub unsafe fn private_copy_six(value: u64, output: *mut u8) {
    let local = value;
    unsafe {
        core::ptr::copy_nonoverlapping((&raw const local).cast::<u8>().add(1), output, 6);
    }
}

#[inline(never)]
pub fn tail_sum(n: u64, sum: u64) -> u64 {
    if n == 0 {
        sum
    } else {
        tail_sum(n - 1, sum.wrapping_add(n))
    }
}

#[inline(never)]
pub fn tail_swap(n: u64, left: u64, right: u64) -> u64 {
    if n == 0 {
        left ^ right.rotate_left(13)
    } else {
        tail_swap(n - 1, right, left.wrapping_add(n))
    }
}

/// # Safety
/// `pointer` is initialized, aligned and writable for every recursion step.
#[inline(never)]
pub unsafe fn tail_effects(n: u64, pointer: *mut u64) -> u64 {
    unsafe {
        if n == 0 {
            *pointer
        } else {
            *pointer = (*pointer).wrapping_add(n);
            tail_effects(n - 1, pointer)
        }
    }
}
