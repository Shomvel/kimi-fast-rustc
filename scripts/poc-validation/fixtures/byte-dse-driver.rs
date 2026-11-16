use byte_dse::*;

#[unsafe(no_mangle)]
pub extern "Rust" fn byte_dse_observe(value: &u64) -> u64 {
    *value
}

fn main() {
    let mut random = 0x8f71_00ff_a54e_1955u64;
    let mut checksum = 0u64;
    for case in 0..20_006 {
        let old = match case {
            0 => 0,
            1 => 1,
            2 => u64::MAX,
            3 => 1 << 63,
            4 => 0x0102_0304_0506_0708,
            5 => 0x807f_ff00_55aa_fe01,
            _ => {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                random
            }
        };
        let next = old.rotate_left(19) ^ 0x1987_4433_27ab_cdef;
        let mut output = !old;
        assert_eq!(halves(&mut output, old, next), next);
        assert_eq!(output, next);
        assert_eq!(bytes(&mut output, old, next), next);
        assert_eq!(output, next);
        let partly_written = (old & 0xffff_ffff_0000_0000) | (next & 0xffff_ffff);
        for complete in [false, true] {
            let expected = if complete { next } else { partly_written };
            assert_eq!(
                incomplete_branch(&mut output, old, next, complete),
                expected
            );
            assert_eq!(output, expected);
            checksum = checksum.wrapping_add(output);
        }
        assert_eq!(
            observed_halfway(&mut output, old, next),
            partly_written ^ next
        );
        assert_eq!(output, next);
        checksum = checksum.rotate_left(7) ^ output ^ partly_written;
    }
    println!("byte dse: 20006 cases passed; checksum={checksum:016x}");
}
