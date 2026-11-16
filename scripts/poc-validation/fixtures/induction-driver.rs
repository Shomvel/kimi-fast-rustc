use induction::*;

fn main() {
    let mut random = 0x8a71_ffff_7000_0155u64;
    let mut checksum = 0u64;
    for case in 0..20_006u64 {
        let start = match case {
            0 => 0,
            1 => 1,
            2 => u64::MAX,
            3 => 1 << 63,
            4 => 127,
            5 => 128,
            _ => {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                random
            }
        };
        let count = (case % 32) as u8;
        let stop = ((start >> 5) % 32) as u8;
        let mut wide = start;
        let mut wide_sum = 0u64;
        let mut byte = start as i8;
        let mut byte_sum = 0i8;
        for _ in 0..count {
            wide_sum = wide_sum.wrapping_add(wide.wrapping_mul(13));
            wide = wide.wrapping_add(0x1_0000_0001);
            byte_sum = byte_sum.wrapping_add(byte.wrapping_mul(11));
            byte = byte.wrapping_sub(17);
        }
        let result = products(start, count);
        let byte_result = byte_products(start as i8, count);
        assert_eq!(result, wide_sum ^ wide);
        assert_eq!(byte_result, byte_sum ^ byte);
        let mut expected_history = [start ^ 0x91; 32];
        let mut history = expected_history;
        let mut trace_index = start;
        let mut trace_sum = 0u64;
        let written = count.min(stop);
        for slot in expected_history.iter_mut().take(usize::from(written)) {
            let product = trace_index.wrapping_mul(23);
            *slot = product;
            trace_sum = trace_sum.wrapping_add(product);
            trace_index = trace_index.wrapping_sub(0x1000_0001);
        }
        let trace = early_exit(start, count, stop, &mut history);
        assert_eq!(trace, trace_sum ^ trace_index ^ u64::from(written));
        assert_eq!(history, expected_history);
        checksum = checksum.rotate_left(7) ^ result ^ byte_result as u64 ^ trace;
        for value in history {
            checksum = checksum.wrapping_add(value);
        }
    }
    println!("induction: 20006 cases passed; checksum={checksum:016x}");
}
