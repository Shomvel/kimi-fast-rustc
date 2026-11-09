//! Direct intrinsics and primitive methods with runtime integer operands.

macro_rules! probes {
    ($function:ident, $integer:ty) => {
        #[inline(never)]
        pub fn $function(
            left: $integer,
            right: $integer,
        ) -> ($integer, $integer, $integer, $integer) {
            let left = std::hint::black_box(left);
            let right = std::hint::black_box(right);
            (
                std::intrinsics::saturating_add(left, right),
                std::intrinsics::saturating_sub(left, right),
                left.saturating_add(right),
                left.saturating_sub(right),
            )
        }
    };
}

probes!(saturate_u8, u8);
probes!(saturate_u16, u16);
probes!(saturate_u32, u32);
probes!(saturate_u64, u64);
probes!(saturate_usize, usize);
probes!(saturate_i8, i8);
probes!(saturate_i16, i16);
probes!(saturate_i32, i32);
probes!(saturate_i64, i64);
probes!(saturate_isize, isize);
