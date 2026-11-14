#![no_std]
#[unsafe(no_mangle)]
pub extern "C" fn byte_shift(x: u8) -> u64 { ((x as u64) >> 2) & 63 }
#[unsafe(no_mangle)]
pub extern "C" fn low_shift(x: u64) -> u64 { (x << 3) & !7 }
#[unsafe(no_mangle)]
pub extern "C" fn invert_mask(x: u8) -> u8 { (!(x & 15)) | 240 }
#[unsafe(no_mangle)]
pub extern "C" fn sign_shift(x: i8) -> i8 { ((x | i8::MIN) >> 3) | -32 }
