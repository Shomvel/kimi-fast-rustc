use private_integer_lanes::*;

fn main() {
    let mut state = 0x807f_ff00_55aa_fe01u64;
    let mut checksum = 0u64;
    for i in 0..10_006 {
        let value = match i {
            0 => 0,
            1 => 1,
            2 => u64::MAX,
            3 => 1 << 63,
            4 => 0x807f_ff00_55aa_fe01,
            5 => 0x0102_0304_0506_0708,
            _ => {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state
            }
        };
        let byte = (value ^ (value >> 32)) as u8;
        let old = (value >> 16) as u16;
        let new = (old & 0xff00) | u16::from(byte);
        assert_eq!(byte_at_three(value), (value >> 24) as u8);
        assert_eq!(half_at_two(value), old);
        assert_eq!(signed_half_at_four(value as i64), (value >> 32) as i16);
        assert_eq!(
            snapshot_then_overwrite(value, byte),
            (u32::from(old) << 16) | u32::from(new)
        );
        assert_eq!(disjoint_store(value, byte), old);
        assert_eq!(reconstruct(value), value);
        checksum = checksum.wrapping_add(value.rotate_left(byte.into()));
    }
    println!("private integer lanes: 10006 cases passed; checksum={checksum:016x}");
}
