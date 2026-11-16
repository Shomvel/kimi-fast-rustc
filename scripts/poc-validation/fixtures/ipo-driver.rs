use ipo::*;

#[unsafe(no_mangle)]
pub unsafe extern "Rust" fn ipo_external_write(pointer: *mut u64, value: u64) -> u64 {
    unsafe {
        let before = *pointer;
        *pointer = value;
        before
    }
}

unsafe fn callback(pointer: *mut u64, value: u64) -> u64 {
    unsafe { ipo_external_write(pointer, value) }
}

fn expected_mix(value: u64) -> u64 {
    value.wrapping_mul(0x9e37_79b9).rotate_left(17) ^ value.rotate_right(11)
}

fn expected_sum(mut n: u64, mut sum: u64) -> u64 {
    while n != 0 {
        sum = sum.wrapping_add(n);
        n -= 1;
    }
    sum
}

fn expected_swap(mut n: u64, mut left: u64, mut right: u64) -> u64 {
    while n != 0 {
        let next = left.wrapping_add(n);
        left = right;
        right = next;
        n -= 1;
    }
    left ^ right.rotate_left(13)
}

fn main() {
    let mut state = 0xb4e1_1539_ab07_00d1u64;
    let mut checksum = 0u64;
    for case in 0..10_006 {
        let a = match case {
            0 => 0,
            1 => 1,
            2 => u64::MAX,
            3 => 1 << 63,
            4 => 0x0102_0304_0506_0708,
            5 => 0x807f_ff00_55aa_fe01,
            _ => {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state
            }
        };
        let b = a.rotate_right(19) ^ 0x5539_07aa_81c2_fd60;
        let mut cells = [0x9173_cafe_0a7d_b5e1, !a, 0x15b1_7539_0099_3307];
        let pointer = unsafe { cells.as_mut_ptr().add(1) };
        assert_eq!(unsafe { readnone_reload(pointer, a) }, (a, expected_mix(a)));
        assert_eq!(unsafe { readonly_reload(pointer, a) }, (a, a));
        assert_eq!(unsafe { writing_reload(pointer, a, b) }, (a, b));
        assert_eq!(cells[1], b);
        assert_eq!(unsafe { unknown_reload(pointer, b, a) }, (b, a));
        assert_eq!(cells[1], a);
        assert_eq!(unsafe { indirect_reload(pointer, a, b, callback) }, (a, b));
        assert_eq!(cells[1], b);
        assert_eq!(unsafe { RAW_READER(pointer) }, b);
        assert_eq!(REFERENCE_READER(&a), a);
        assert_eq!(addressed_direct(&b), b);
        assert_eq!(promoted_reads(a, b), a ^ a.wrapping_add(b));
        let c = b.rotate_left(7) ^ 0x00ff_8091_3579_2468;
        assert_eq!(promoted_field_reads(a, b, c), (a ^ b, a ^ b ^ c));
        let mut output = [0x95u8; 16];
        let mut expected = output;
        expected[1..4].copy_from_slice(&a.to_ne_bytes()[2..5]);
        unsafe { private_copy_three(a, output.as_mut_ptr().add(1)) };
        assert_eq!(output, expected);
        // Both copies start at an odd destination. Compare the entire buffer
        // so prefix/suffix overwrites cannot hide behind a value checksum.
        output.fill(0x39);
        expected.fill(0x39);
        expected[1..7].copy_from_slice(&b.to_ne_bytes()[1..7]);
        unsafe { private_copy_six(b, output.as_mut_ptr().add(1)) };
        assert_eq!(output, expected);
        assert_eq!(cells[0], 0x9173_cafe_0a7d_b5e1);
        assert_eq!(cells[2], 0x15b1_7539_0099_3307);
        let n = (case % 63) as u64;
        assert_eq!(tail_sum(n, a), expected_sum(n, a));
        assert_eq!(tail_swap(n, a, b), expected_swap(n, a, b));
        assert_eq!(unsafe { tail_effects(n, pointer) }, expected_sum(n, b));
        assert_eq!(cells[1], expected_sum(n, b));
        checksum = checksum.wrapping_add(cells[1] ^ expected_mix(a) ^ promoted_reads(a, b));
    }
    // Each function must complete far beyond what the 512 KiB test stack can
    // hold as native recursive frames. Compare final values and side effects.
    for n in [100_000, 100_001, 250_000] {
        assert_eq!(tail_sum(n, 7), expected_sum(n, 7));
        assert_eq!(tail_swap(n, 11, 19), expected_swap(n, 11, 19));
        let mut value = 23;
        assert_eq!(unsafe { tail_effects(n, &mut value) }, expected_sum(n, 23));
        assert_eq!(value, expected_sum(n, 23));
        checksum ^= value;
    }
    println!("ipo: 10006 cases and 100000/100001/250000-depth recursions passed; checksum={checksum:016x}");
}
