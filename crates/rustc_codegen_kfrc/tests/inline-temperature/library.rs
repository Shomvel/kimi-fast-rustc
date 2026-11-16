fn mix(mut value: u64, key: u64) -> u64 {
    macro_rules! step {
        () => {
            value = value.wrapping_mul(key).wrapping_add(key) ^ key;
        };
    }
    step!();
    step!();
    step!();
    step!();
    step!();
    step!();
    step!();
    step!();
    step!();
    step!();
    step!();
    step!();
    step!();
    step!();
    step!();
    step!();
    step!();
    step!();
    value
}

#[inline(never)]
pub fn ordinary(value: u64, key: u64) -> u64 {
    mix(value, key)
}

#[inline(never)]
pub fn cyclic(mut value: u64, key: u64, count: usize) -> u64 {
    for index in 0..count {
        value = mix(value ^ index as u64, key);
    }
    value
}

fn guarded(value: u64, key: u64) -> u64 {
    if key == 0 {
        panic!("cold value {}", mix(value, key));
    }
    mix(value, key)
}

#[inline(never)]
pub fn guarded_loop(mut value: u64, key: u64, count: usize) -> u64 {
    for index in 0..count {
        value = guarded(value ^ index as u64, key);
    }
    value
}

unsafe extern "C" {
    fn observe_cold_arguments(
        a: i8,
        b: u16,
        c: u64,
        d: u64,
        e: u64,
        f: u64,
        stack: u64,
        x_bits: u32,
        y_bits: u64,
    ) -> u64;
}

#[inline(never)]
pub fn guarded_abi(a: i8, b: u16, seed: u64, x_bits: u32, y_bits: u64, fail: bool) -> u64 {
    if fail {
        let observed = unsafe {
            observe_cold_arguments(
                a,
                b,
                seed,
                seed ^ 3,
                seed ^ 5,
                seed ^ 7,
                seed ^ 11,
                x_bits,
                y_bits,
            )
        };
        panic!("cold ABI {observed:016x}");
    }
    seed
}
