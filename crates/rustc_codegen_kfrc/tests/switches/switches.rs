//! Integer dispatch through both machine layers; the driver owns the oracle.
#![no_std]

#[inline(never)]
pub fn dense8(value: u8) -> u64 {
    match value {
        10 => 37,
        11 => 11,
        12 => 92,
        13 => 4,
        14 => 71,
        15 => 55,
        _ => 777,
    }
}

#[inline(never)]
pub fn signed8(value: i8) -> u64 {
    match value {
        -128 => 41,
        -3 => 27,
        -2 => 91,
        -1 => 5,
        0 => 66,
        1 => 80,
        127 => 19,
        _ => 999,
    }
}

#[inline(never)]
pub fn sparse64(value: u64) -> u64 {
    match value {
        0 => 13,
        99 => 72,
        4096 => 45,
        0xffff_ffff => 34,
        0x1_0000_0000 => 91,
        0x8000_0000_0000_0000 => 83,
        0xffff_ffff_ffff_ffff => 51,
        _ => 888,
    }
}

#[inline(never)]
pub fn shared_phi(value: u32, salt: u64) -> u64 {
    let extra = match value {
        3 => 17,
        4 => 9,
        5 => salt,
        6 => 81,
        7 => 33,
        _ => 101,
    };
    extra ^ salt
}

#[inline(never)]
fn event(value: u64) -> u64 {
    core::hint::black_box(value).wrapping_add(11)
}

#[inline(never)]
pub fn fixed_event(value: u64) -> u64 {
    if value == 1 {
        91
    } else if value == 2 {
        73
    } else {
        let added = event(value);
        if value == 3 {
            added ^ 27
        } else if value == 4 {
            added ^ 41
        } else {
            added ^ 88
        }
    }
}

#[inline(never)]
pub fn loop_dispatch(mut value: u64, count: u32) -> u64 {
    let mut result = 0u64;
    let mut index = 0;
    while index < count {
        result = result.wrapping_add(match value & 7 {
            0 => 31,
            1 => 19,
            2 => 73,
            3 => 11,
            4 => 97,
            5 => 42,
            _ => 88,
        });
        value = value.rotate_left(5).wrapping_add(index as u64);
        index += 1;
    }
    result
}
