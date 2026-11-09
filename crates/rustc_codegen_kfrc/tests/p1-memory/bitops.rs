//! Runtime shift operands exercise all supported integer widths.

macro_rules! probes {
    ($rotate:ident, $funnel:ident, $integer:ty, $unsigned:ty) => {
        #[inline(never)]
        pub fn $rotate(value: $integer, shift: u32) -> ($integer, $integer) {
            let value = std::hint::black_box(value);
            let shift = std::hint::black_box(shift);
            (value.rotate_left(shift), value.rotate_right(shift))
        }

        #[inline(never)]
        pub fn $funnel(left: $integer, right: $integer, shift: u32) -> ($integer, $integer) {
            // Rust's intrinsic accepts unsigned integers. Reinterpret signed
            // inputs at the same width and cast its resulting bit pattern back.
            let left = std::hint::black_box(left) as $unsigned;
            let right = std::hint::black_box(right) as $unsigned;
            let shift = std::hint::black_box(shift);
            // Callers restrict shifts to 0..BITS for the unchecked contract.
            unsafe {
                (
                    std::intrinsics::unchecked_funnel_shl(left, right, shift) as $integer,
                    std::intrinsics::unchecked_funnel_shr(left, right, shift) as $integer,
                )
            }
        }
    };
}

probes!(rotate_u8, funnel_u8, u8, u8);
probes!(rotate_u16, funnel_u16, u16, u16);
probes!(rotate_u32, funnel_u32, u32, u32);
probes!(rotate_u64, funnel_u64, u64, u64);
probes!(rotate_usize, funnel_usize, usize, usize);
probes!(rotate_i8, funnel_i8, i8, u8);
probes!(rotate_i16, funnel_i16, i16, u16);
probes!(rotate_i32, funnel_i32, i32, u32);
probes!(rotate_i64, funnel_i64, i64, u64);
probes!(rotate_isize, funnel_isize, isize, usize);
