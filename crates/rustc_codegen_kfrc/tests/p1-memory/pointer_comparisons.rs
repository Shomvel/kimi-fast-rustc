//! Pointer-producing intrinsics retain the source-level pointer type.

#![allow(ambiguous_wide_pointer_comparisons)]

macro_rules! relations {
    ($left:expr, $right:expr) => {{
        let left = $left;
        let right = $right;
        (left == right) as u8
            | (((left != right) as u8) << 1)
            | (((left < right) as u8) << 2)
            | (((left <= right) as u8) << 3)
            | (((left > right) as u8) << 4)
            | (((left >= right) as u8) << 5)
    }};
}

#[inline(never)]
pub fn thin(index: usize, other: usize) -> (u8, u8, u8, u8, bool, bool) {
    let mut values = [3_u64, 5, 7, 11, 13, 17, 19, 23];
    let shared = std::hint::black_box(values.as_ptr());
    let mutable = std::hint::black_box(values.as_mut_ptr());
    let const_left = shared.wrapping_add(index);
    let const_right = shared.wrapping_offset(other as isize);
    let mut_left = mutable.wrapping_add(index);
    let mut_right = mutable.wrapping_offset(other as isize);
    let tagged_const = (const_left as usize | 3) as *const u64;
    let tagged_mut = (mut_left as usize | 5) as *mut u64;
    let masked_const = tagged_const.mask(!7);
    let masked_mut = tagged_mut.mask(!7);
    // Casting across element types is explicit in MIR; comparison operands
    // then share the selected pointer type and preserve the same address.
    let bytes_left = const_left.cast::<u8>();
    let bytes_right = mut_right.cast::<u8>().cast_const();
    (
        relations!(const_left, const_right),
        relations!(mut_left, mut_right),
        relations!(bytes_left, bytes_right),
        relations!(const_left, mut_right.cast_const()),
        masked_const == const_left,
        masked_mut == mut_left,
    )
}

#[inline(never)]
pub fn wide(
    left_index: usize,
    left_len: usize,
    right_index: usize,
    right_len: usize,
) -> (u8, u8, bool) {
    let mut values = [3_u64, 5, 7, 11, 13, 17, 19, 23];
    let base = std::hint::black_box(values.as_mut_ptr());
    let left: *const [u64] =
        std::ptr::slice_from_raw_parts(base.wrapping_add(left_index), left_len);
    let right: *mut [u64] =
        std::ptr::slice_from_raw_parts_mut(base.wrapping_add(right_index), right_len);
    let tagged = std::ptr::slice_from_raw_parts((base as usize | 3) as *const u64, left_len);
    let masked = tagged.mask(!7);
    let original = std::ptr::slice_from_raw_parts(base.cast_const(), left_len);
    (
        relations!(left, right.cast_const()),
        relations!(left.cast_mut(), right),
        masked == original,
    )
}
