#![crate_type = "rlib"]

#[inline(never)]
pub fn overwritten_pair(out: &mut (u64, u64), a: u64, b: u64) -> u64 {
    out.0 = a;
    out.1 = b;
    let captured = out.0;
    out.0 = a ^ b;
    out.0 = a.wrapping_add(b);
    captured ^ out.0 ^ out.1
}

#[inline(never)]
pub fn forward_disjoint_fields(out: &mut [u64; 4], a: u64, b: u64) -> u64 {
    out[0] = a;
    out[1] = b;
    let first = out[0];
    out[3] = a ^ b;
    let second = out[0];
    out[2] = first.wrapping_add(second);
    first ^ second ^ out[1] ^ out[2] ^ out[3]
}

/// # Safety
/// `p` and `q` must each point to an initialized, aligned, writable u64.
/// They may denote the same word; no references are made from them.
#[inline(never)]
pub unsafe fn raw_may_alias(p: *mut u64, q: *mut u64, a: u64, b: u64) -> (u64, u64, u64) {
    unsafe {
        *p = a;
        let first = *p;
        *q = b;
        let second = *p;
        *p = first ^ second;
        (first, second, *q)
    }
}

/// # Safety
/// `out` must point to an initialized, aligned, writable u64.
#[inline(never)]
pub unsafe fn raw_repeated_overwrite(out: *mut u64, a: u64, b: u64) -> u64 {
    unsafe {
        *out = a;
        *out = b;
        *out = a ^ b;
        *out
    }
}

/// # Safety
/// `out` must point to an initialized, aligned, writable u64. The byte access
/// remains inside the same word and must affect the following whole-word load.
#[inline(never)]
pub unsafe fn overlapping_byte(out: *mut u64, a: u64, byte: u8) -> u64 {
    unsafe {
        *out = a;
        let old = *out;
        *(out as *mut u8).add(3) = byte;
        old ^ *out
    }
}

#[inline(never)]
fn observe_pair(value: &(u64, u64)) -> u64 {
    value.0.wrapping_add(value.1)
}

#[inline(never)]
pub fn observed_before_overwrite(out: &mut (u64, u64), a: u64, b: u64) -> u64 {
    out.0 = a;
    out.1 = b;
    let observed = observe_pair(out);
    out.0 = a ^ b;
    observed
}

#[inline(never)]
pub fn conditional_overwrite(out: &mut (u64, u64), condition: bool, a: u64, b: u64) -> u64 {
    out.0 = a;
    if condition {
        out.0 = b;
    }
    out.1 = out.0 ^ a;
    out.0
}

#[inline(never)]
pub fn both_branches_overwrite(out: &mut (u64, u64), condition: bool, a: u64, b: u64) -> u64 {
    out.0 = a;
    out.1 = b;
    if condition {
        out.0 = b.wrapping_add(1);
    } else {
        out.0 = a ^ b;
    }
    out.1 ^= out.0;
    out.0
}

#[inline(never)]
pub fn observed_branch_overwrite(out: &mut (u64, u64), condition: bool, a: u64, b: u64) -> u64 {
    out.0 = a;
    out.1 = b;
    if condition {
        let observed = observe_pair(out);
        out.0 = b;
        observed
    } else {
        out.0 = a.wrapping_add(b);
        a ^ b
    }
}

// The common LLVM-built driver supplies these callees. Their bodies are not
// available for library inlining, so the actual call-site borrow carriers and
// memory effects remain exercised by both library backends.
unsafe extern "Rust" {
    fn generic_memory_observe_pair(value: &(u64, u64)) -> u64;
    fn generic_memory_observe_word(value: &u64) -> u64;
    fn generic_memory_observe_nested(value: &(u64, (u64, u64))) -> u64;
    fn generic_memory_mutate_cell(value: &std::cell::UnsafeCell<u64>, next: u64) -> u64;
    fn generic_memory_observe_and_write(protected: &u64, sibling: *mut u64, next: u64) -> u64;
}

#[inline(never)]
pub fn frozen_whole_object_reload(out: &mut (u64, u64), a: u64, b: u64) -> (u64, u64) {
    out.0 = a;
    out.1 = b;
    let observed = unsafe { generic_memory_observe_pair(out) };
    (observed, out.1)
}

#[inline(never)]
pub fn frozen_exact_field_reload(out: &mut (u64, u64), value: u64) -> (u64, u64) {
    out.1 = value;
    let observed = unsafe { generic_memory_observe_word(&out.1) };
    (observed, out.1)
}

#[inline(never)]
pub fn frozen_nested_reload(out: &mut (u64, (u64, u64)), a: u64, b: u64) -> (u64, u64, u64) {
    out.0 = a ^ b;
    out.1 .0 = a;
    out.1 .1 = b;
    let observed = unsafe { generic_memory_observe_nested(out) };
    (observed, out.1 .0, out.1 .1)
}

/// # Safety
/// `out` must point to a live, aligned, initialized, writable pair. No access
/// may conflict with the shared reborrow during the observer call.
#[inline(never)]
pub unsafe fn frozen_raw_reborrow_reload(out: *mut (u64, u64), a: u64, b: u64) -> (u64, u64) {
    unsafe {
        (*out).0 = a;
        (*out).1 = b;
        let observed = generic_memory_observe_pair(&*out);
        (observed, (*out).1)
    }
}

#[inline(never)]
pub fn unsafe_cell_reload(value: &std::cell::UnsafeCell<u64>, next: u64) -> (u64, u64, u64) {
    let before = unsafe { *value.get() };
    let observed = unsafe { generic_memory_mutate_cell(value, next) };
    (before, observed, unsafe { *value.get() })
}

/// # Safety
/// `out` must point to a live, aligned, initialized, writable pair. The callee
/// reads the shared first field and writes only the disjoint second field.
#[inline(never)]
pub unsafe fn protected_sibling_mutation(
    out: *mut (u64, u64),
    a: u64,
    b: u64,
    next: u64,
) -> (u64, u64, u64) {
    unsafe {
        (*out).0 = a;
        (*out).1 = b;
        let observed =
            generic_memory_observe_and_write(&(*out).0, std::ptr::addr_of_mut!((*out).1), next);
        (observed, (*out).0, (*out).1)
    }
}
