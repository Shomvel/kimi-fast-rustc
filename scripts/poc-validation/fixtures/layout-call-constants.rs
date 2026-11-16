#![crate_type = "rlib"]
#![feature(core_intrinsics)]
#![allow(internal_features)]

// Every intrinsic below receives the incoming dynamic length. The literals
// constrain its control path; they are deliberately not intrinsic operands.
macro_rules! guarded_equalities {
    ($($name:ident => $length:literal),+ $(,)?) => {
        $(
            /// Return 0/1 for a matching length and 2 for an untaken guard.
            ///
            /// # Safety
            /// When `length` matches this function's guard, both pointers must
            /// satisfy `compare_bytes` for its entire readable, initialized
            /// range. The ranges may alias or overlap. For length zero use
            /// non-null aligned pointers. Otherwise no pointer is accessed.
            #[inline(never)]
            pub unsafe fn $name(left: *const u8, right: *const u8, length: usize) -> u8 {
                if length == $length {
                    // The caller supplies the complete range on this path.
                    u8::from(unsafe { core::intrinsics::compare_bytes(left, right, length) } == 0)
                } else {
                    2
                }
            }
        )+
    };
}

guarded_equalities! {
    equal_zero => 0,
    equal_one => 1,
    equal_two => 2,
    equal_three => 3,
    equal_four => 4,
    equal_five => 5,
    equal_six => 6,
    equal_seven => 7,
    equal_eight => 8,
    equal_fifteen => 15,
    equal_sixteen => 16,
    equal_seventeen => 17,
    equal_thirty_one => 31,
    equal_thirty_two => 32,
    equal_thirty_three => 33,
}

/// # Safety
/// Both pointers satisfy `compare_bytes` for three bytes if `length == 3`.
/// Other lengths do not access either pointer.
#[inline(never)]
pub unsafe fn not_equal_guard(left: *const u8, right: *const u8, length: usize) -> u8 {
    if length != 3 {
        2
    } else {
        u8::from(unsafe { core::intrinsics::compare_bytes(left, right, length) } != 0)
    }
}

/// # Safety
/// Both pointers satisfy `compare_bytes` for seven bytes if `length == 7`.
/// Other lengths do not access either pointer.
#[inline(never)]
pub unsafe fn inverted_guard(left: *const u8, right: *const u8, length: usize) -> u8 {
    let different_length = length != 7;
    if !different_length {
        u8::from(unsafe { core::intrinsics::compare_bytes(left, right, length) } == 0)
    } else {
        2
    }
}

/// # Safety
/// All three pointers satisfy `compare_bytes` for seven bytes if `length == 7`.
/// Their readable ranges may alias or overlap. Other lengths access nothing.
#[inline(never)]
pub unsafe fn two_calls(
    left: *const u8,
    first: *const u8,
    second: *const u8,
    length: usize,
) -> u8 {
    if length == 7 {
        let first_equal = unsafe { core::intrinsics::compare_bytes(left, first, length) } == 0;
        let second_unequal = unsafe { core::intrinsics::compare_bytes(left, second, length) } != 0;
        u8::from(first_equal) | (u8::from(second_unequal) << 1)
    } else {
        4
    }
}

/// # Safety
/// Both pointers satisfy `compare_bytes` for the complete dynamic `length`.
/// For zero length the pointers must still be non-null and aligned.
#[inline(never)]
pub unsafe fn sibling_fallback(
    left: *const u8,
    right: *const u8,
    length: usize,
    first_branch: bool,
) -> u8 {
    if first_branch {
        if length == 3 {
            return u8::from(unsafe { core::intrinsics::compare_bytes(left, right, length) } == 0);
        }
    } else if length == 16 {
        return 2 + u8::from(unsafe { core::intrinsics::compare_bytes(left, right, length) } == 0);
    }
    // Neither sibling fact holds at this merged fallback.
    4 + u8::from(unsafe { core::intrinsics::compare_bytes(left, right, length) } == 0)
}

/// # Safety
/// All three pointers satisfy `compare_bytes` for the complete dynamic length.
/// All may alias or overlap. Zero-length pointers must be non-null and aligned.
#[inline(never)]
pub unsafe fn merged_fallback(
    left: *const u8,
    right: *const u8,
    other: *const u8,
    length: usize,
) -> bool {
    let selected = if length == 7 { left } else { right };
    // This call is after the merge, so its length remains dynamic.
    unsafe { core::intrinsics::compare_bytes(selected, other, length) == 0 }
}

/// # Safety
/// Both pointers satisfy `compare_bytes` for the complete dynamic length.
/// The nonzero return magnitude is unspecified; only its sign is meaningful.
#[inline(never)]
pub unsafe fn ordering(left: *const u8, right: *const u8, length: usize) -> i32 {
    if length == 3 {
        unsafe { core::intrinsics::compare_bytes(left, right, length) }
    } else if length == 7 {
        unsafe { core::intrinsics::compare_bytes(left, right, length) }
    } else if length == 16 {
        unsafe { core::intrinsics::compare_bytes(left, right, length) }
    } else if length == 32 {
        unsafe { core::intrinsics::compare_bytes(left, right, length) }
    } else if length == 33 {
        unsafe { core::intrinsics::compare_bytes(left, right, length) }
    } else {
        unsafe { core::intrinsics::compare_bytes(left, right, length) }
    }
}

/// # Safety
/// Both pointers satisfy `compare_bytes` for every length in `lengths`.
/// Zero-length pointers must be non-null and aligned. The length slice is a
/// separate live initialized allocation that is not mutated during the call.
#[inline(never)]
pub unsafe fn changing_lengths(left: *const u8, right: *const u8, lengths: &[usize]) -> u64 {
    let mut result = 0u64;
    for &length in lengths {
        let equal = if length == 3 {
            unsafe { core::intrinsics::compare_bytes(left, right, length) == 0 }
        } else if length == 7 {
            unsafe { core::intrinsics::compare_bytes(left, right, length) == 0 }
        } else if length == 16 {
            unsafe { core::intrinsics::compare_bytes(left, right, length) == 0 }
        } else if length == 32 {
            unsafe { core::intrinsics::compare_bytes(left, right, length) == 0 }
        } else {
            unsafe { core::intrinsics::compare_bytes(left, right, length) == 0 }
        };
        result = result.rotate_left(5) ^ (length as u64).wrapping_mul(17) ^ u64::from(equal);
    }
    result
}

/// # Safety
/// Both pointers satisfy `compare_bytes` for seven bytes if any slice element
/// is seven. Otherwise no pointer is accessed. `lengths` remains initialized
/// and unmodified during the call and is disjoint from any writable storage.
#[inline(never)]
pub unsafe fn refreshed_guard(left: *const u8, right: *const u8, lengths: &[usize]) -> u64 {
    let mut result = 0u64;
    for &length in lengths {
        let value = if length == 7 {
            u64::from(unsafe { core::intrinsics::compare_bytes(left, right, length) } == 0)
        } else {
            2
        };
        result = result.rotate_left(3) ^ (length as u64).wrapping_mul(13) ^ value;
    }
    result
}
