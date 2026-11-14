//! Returning signatures with terminal bodies exercise the actual inline seam.
extern "C" {
    fn record_event(value: u64) -> u64;
    fn stop_process(value: u64) -> !;
}

#[inline(always)]
fn inline_halt(value: u64) -> u64 {
    unsafe {
        record_event(value);
        stop_process(value.wrapping_add(1))
    }
}

#[inline(never)]
fn closed_halt(value: u64) -> u64 {
    unsafe {
        record_event(value);
        stop_process(value.wrapping_add(1))
    }
}

#[no_mangle]
pub extern "C" fn inline_exit(value: u64) -> u64 {
    unsafe {
        record_event(value.wrapping_sub(1));
    }
    let result = inline_halt(value);
    unsafe {
        record_event(result.wrapping_add(100));
    }
    result.wrapping_add(1)
}

#[no_mangle]
pub extern "C" fn closed_exit(value: u64) -> u64 {
    unsafe {
        record_event(value.wrapping_sub(1));
    }
    let result = closed_halt(value);
    unsafe {
        record_event(result.wrapping_add(100));
    }
    result.wrapping_add(1)
}

/// The driver only supplies values satisfying this unsafe precondition.
#[no_mangle]
pub unsafe extern "C" fn defined_guard(value: u64) -> u64 {
    if value & 7 == 0 {
        unsafe { std::hint::unreachable_unchecked() }
    }
    value.wrapping_mul(0x5851f42d4c957f2d).rotate_left(11)
}
