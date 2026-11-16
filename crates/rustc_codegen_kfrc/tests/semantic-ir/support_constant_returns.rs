//! Narrow ABI lanes are read by the LLVM-compiled caller as wider integers.
macro_rules! constants {
    ($ty:ty, $low:ident, $high:ident, $middle:ident, $value:expr) => {
        #[inline(never)]
        pub fn $low() -> $ty {
            <$ty>::MIN
        }
        #[inline(never)]
        pub fn $high() -> $ty {
            <$ty>::MAX
        }
        #[inline(never)]
        pub fn $middle() -> $ty {
            $value
        }
    };
}

constants!(u8, u8_low, u8_high, u8_middle, 0x80);
constants!(i8, i8_low, i8_high, i8_middle, -1);
constants!(u16, u16_low, u16_high, u16_middle, 0x8000);
constants!(i16, i16_low, i16_high, i16_middle, -1);
constants!(u32, u32_low, u32_high, u32_middle, 0x8000_0000);
constants!(i32, i32_low, i32_high, i32_middle, -1);

#[inline(never)]
pub fn yes() -> bool {
    true
}

#[inline(never)]
pub fn no() -> bool {
    false
}

#[inline(never)]
pub fn shared(saved: &mut i8) -> i8 {
    *saved = -1;
    -1
}

#[inline(never)]
pub fn pair() -> (i8, u16) {
    (-128, 0x8000)
}

#[inline(never)]
pub fn dynamic_signed(value: i8) -> i8 {
    value
}

#[inline(never)]
pub fn dynamic_unsigned(value: u16) -> u16 {
    value
}

#[inline(never)]
pub fn mixed(value: i8) -> (u16, i8) {
    (0x8000, value)
}
