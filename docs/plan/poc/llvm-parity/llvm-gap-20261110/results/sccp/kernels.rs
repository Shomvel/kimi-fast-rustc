#![no_std]

#[unsafe(no_mangle)]
pub extern "C" fn constant_merge(flag: bool) -> u64 {
    let value = if flag { 7u64 } else { 7u64 };
    value * 6
}

#[unsafe(no_mangle)]
pub extern "C" fn constant_branch(flag: bool) -> u64 {
    let value = if flag { 7u64 } else { 7u64 };
    if value == 7 { 42 } else { 99 }
}

#[unsafe(no_mangle)]
pub extern "C" fn narrow_merge(flag: bool) -> u8 {
    let value = if flag { 255u8 } else { 255u8 };
    value + 2
}

#[unsafe(no_mangle)]
pub extern "C" fn changing_loop(limit: u64) -> u64 {
    let mut i = 0u64;
    let mut value = 42u64;
    while i < limit {
        value = if i & 1 == 0 { 42 } else { 42 };
        i += 1;
    }
    value + i
}

#[unsafe(no_mangle)]
pub extern "C" fn dynamic_merge(flag: bool, value: u64) -> u64 {
    let merged = if flag { 7 } else { value };
    merged + 1
}
