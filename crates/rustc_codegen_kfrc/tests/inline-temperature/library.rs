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
