//! Large arrays cross copies, moves and ABI boundaries without observing padding.
#[repr(C)]
pub struct Packet {
    pub head: u8,
    pub table: [u8; 64],
    pub tail: u64,
}

#[inline(never)]
pub fn make(seed: u8) -> [u8; 256] {
    let mut result = [0; 256];
    for (index, byte) in result.iter_mut().enumerate() {
        *byte = seed.wrapping_add((index as u8).wrapping_mul(7));
    }
    result
}

#[inline(never)]
pub fn copy_then_mutate(source: &mut [u8; 256]) -> [u8; 256] {
    let snapshot = *source;
    *source = [99; 256];
    snapshot
}

#[inline(never)]
pub fn wrap(seed: u8) -> Packet {
    let mut table = [0; 64];
    for (index, byte) in table.iter_mut().enumerate() {
        *byte = seed.wrapping_add(index as u8);
    }
    Packet { head: seed, table, tail: 0x1234_5678_9abc_def0 }
}

#[inline(never)]
pub fn array_phi(mut value: [u8; 64], count: usize) -> [u8; 64] {
    let mut previous = [17; 64];
    for _ in 0..count {
        let next = value;
        value = previous;
        previous = next;
    }
    value
}

#[inline(never)]
pub fn update(value: [u8; 64], index: usize, byte: u8) -> [u8; 64] {
    let mut copied = value;
    copied[index] = byte;
    copied
}

#[inline(never)]
pub fn observe(value: [u8; 64]) -> u8 { value[7] ^ value[61] }
