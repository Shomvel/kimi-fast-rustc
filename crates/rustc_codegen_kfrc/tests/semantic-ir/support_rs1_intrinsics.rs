//! Defined-input intrinsic probes. The crate enables core_intrinsics;
//! unchecked zero/count violations belong only in interpreter tests.

macro_rules! probes {
    ($counts:ident, $reverse:ident, $rotate:ident, $saturate:ident, $ty:ty) => {
        #[inline(never)]
        pub fn $counts(value: $ty) -> (u32, u32, u32) {
            (
                value.count_ones(),
                value.leading_zeros(),
                value.trailing_zeros(),
            )
        }
        #[inline(never)]
        pub fn $reverse(value: $ty) -> ($ty, $ty) {
            (value.swap_bytes(), value.reverse_bits())
        }
        #[inline(never)]
        pub fn $rotate(value: $ty, amount: u32) -> ($ty, $ty) {
            (value.rotate_left(amount), value.rotate_right(amount))
        }
        #[inline(never)]
        pub fn $saturate(left: $ty, right: $ty) -> ($ty, $ty) {
            (left.saturating_add(right), left.saturating_sub(right))
        }
    };
}
probes!(counts_u8, reverse_u8, rotate_u8, saturate_u8, u8);
probes!(counts_u16, reverse_u16, rotate_u16, saturate_u16, u16);
probes!(counts_u32, reverse_u32, rotate_u32, saturate_u32, u32);
probes!(counts_u64, reverse_u64, rotate_u64, saturate_u64, u64);
probes!(
    counts_usize,
    reverse_usize,
    rotate_usize,
    saturate_usize,
    usize
);
probes!(counts_i8, reverse_i8, rotate_i8, saturate_i8, i8);
probes!(counts_i16, reverse_i16, rotate_i16, saturate_i16, i16);
probes!(counts_i32, reverse_i32, rotate_i32, saturate_i32, i32);
probes!(counts_i64, reverse_i64, rotate_i64, saturate_i64, i64);
probes!(
    counts_isize,
    reverse_isize,
    rotate_isize,
    saturate_isize,
    isize
);

macro_rules! funnel {
    ($name:ident, $ty:ty) => {
        /// Caller must provide amount strictly below the integer bit width.
        #[inline(never)]
        pub unsafe fn $name(hi: $ty, lo: $ty, amount: u32) -> ($ty, $ty) {
            unsafe {
                (
                    core::intrinsics::unchecked_funnel_shl(hi, lo, amount),
                    core::intrinsics::unchecked_funnel_shr(hi, lo, amount),
                )
            }
        }
    };
}
funnel!(funnel_u8, u8);
funnel!(funnel_u16, u16);
funnel!(funnel_u32, u32);
funnel!(funnel_u64, u64);
funnel!(funnel_usize, usize);

#[inline(never)]
pub fn counts_nonzero(value: core::num::NonZeroU64) -> (u32, u32) {
    unsafe {
        (
            core::intrinsics::ctlz_nonzero(value.get()),
            core::intrinsics::cttz_nonzero(value.get()),
        )
    }
}

#[inline(never)]
pub fn constants() -> (u32, u64, u32, i16) {
    (
        0xf0f0u64.count_ones(),
        0x0102030405060708u64.swap_bytes(),
        0x12345678u32.reverse_bits(),
        i16::MAX.saturating_add(1),
    )
}

/// An equivalent of the small leading-zero helper which expanded to hundreds
/// of scalar statements before the dedicated operation was retained.
#[inline(never)]
pub fn bytes_for_varint(value: u64) -> usize {
    ((64 - (value | 1).leading_zeros()) as usize + 6) / 7
}

#[inline(never)]
pub fn inline_len(value: u64) -> u32 {
    value.leading_zeros() / 8
}

/// The unused type parameter makes this eligible for definition R; direct
/// intrinsic calls exercise its producer without requiring a generic width.
#[inline(never)]
pub fn definition_intrinsics<T>(value: u64, amount: u32) -> u64 {
    u64::from(core::intrinsics::ctpop(value))
        ^ core::intrinsics::rotate_left(value, amount)
        ^ core::intrinsics::saturating_add(value, 17)
}

/// Keep the generic instantiation inside the library compiled by kfrc.
#[inline(never)]
pub fn definition_probe(value: u64, amount: u32) -> u64 {
    definition_intrinsics::<u8>(value, amount)
}
