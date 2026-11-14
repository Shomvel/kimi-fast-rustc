#![crate_type = "rlib"]

#[inline(never)]
pub fn byte_at_three(mut value: u64) -> u8 {
    let address = &mut value as *mut u64 as *mut u8;
    // The byte lies inside the fully initialized private integer.
    unsafe { *address.add(3) }
}

#[inline(never)]
pub fn half_at_two(mut value: u64) -> u16 {
    let address = &mut value as *mut u64 as *mut u16;
    // The second u16 is aligned and lies inside the initialized u64.
    unsafe { *address.add(1) }
}

#[inline(never)]
pub fn signed_half_at_four(mut value: i64) -> i16 {
    let address = &mut value as *mut i64 as *mut i16;
    // The third i16 is aligned and lies inside the initialized i64.
    unsafe { *address.add(2) }
}

#[inline(never)]
pub fn snapshot_then_overwrite(mut value: u64, byte: u8) -> u32 {
    let address = &mut value as *mut u64 as *mut u8;
    // All accesses are in bounds; the loaded scalar must keep its old bits
    // when a later store changes part of the same private word.
    unsafe {
        let old = *(address.add(2) as *const u16);
        *address.add(2) = byte;
        let new = *(address.add(2) as *const u16);
        (u32::from(old) << 16) | u32::from(new)
    }
}

#[inline(never)]
pub fn disjoint_store(mut value: u64, byte: u8) -> u16 {
    let address = &mut value as *mut u64 as *mut u8;
    // Changing byte seven cannot change bytes two and three.
    unsafe {
        *address.add(7) = byte;
        *(address.add(2) as *const u16)
    }
}

#[inline(never)]
pub fn reconstruct(mut value: u64) -> u64 {
    let address = &mut value as *mut u64 as *mut u8;
    // Read exactly the eight initialized bytes, in their original order.
    unsafe {
        u64::from(*address)
            | (u64::from(*address.add(1)) << 8)
            | (u64::from(*address.add(2)) << 16)
            | (u64::from(*address.add(3)) << 24)
            | (u64::from(*address.add(4)) << 32)
            | (u64::from(*address.add(5)) << 40)
            | (u64::from(*address.add(6)) << 48)
            | (u64::from(*address.add(7)) << 56)
    }
}
