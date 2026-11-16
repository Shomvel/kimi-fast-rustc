#![no_std]

#[unsafe(no_mangle)]
pub extern "C" fn nested_add(x: u64) -> u64 { (x + 13) + 29 }

#[unsafe(no_mangle)]
pub extern "C" fn nested_mul(x: u64) -> u64 { (x * 3) * 7 }

#[unsafe(no_mangle)]
pub extern "C" fn nested_and(x: u64) -> u64 { (x & 0xff00ff) & 0x0f000f }

#[unsafe(no_mangle)]
pub extern "C" fn nested_or(x: u64) -> u64 { (x | 0x5500) | 0xaa00 }

#[unsafe(no_mangle)]
pub extern "C" fn nested_xor(x: u64) -> u64 { (x ^ 0x5500) ^ 0xaa00 }

#[unsafe(no_mangle)]
pub extern "C" fn reversed_add(x: u64, y: u64) -> u64 { (x + y) ^ (y + x) }
