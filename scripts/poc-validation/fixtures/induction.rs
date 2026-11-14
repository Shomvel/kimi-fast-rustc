#![crate_type = "rlib"]

#[inline(never)]
pub fn products(mut index: u64, count: u8) -> u64 {
    let mut iteration = 0u8;
    let mut sum = 0u64;
    while iteration < count {
        sum = sum.wrapping_add(std::hint::black_box(index.wrapping_mul(13)));
        index = index.wrapping_add(0x1_0000_0001);
        iteration = iteration.wrapping_add(1);
    }
    sum ^ index
}

#[inline(never)]
pub fn byte_products(mut index: i8, count: u8) -> i8 {
    let mut iteration = 0u8;
    let mut sum = 0i8;
    while iteration < count {
        sum = sum.wrapping_add(std::hint::black_box(index.wrapping_mul(11)));
        index = index.wrapping_sub(17);
        iteration = iteration.wrapping_add(1);
    }
    sum ^ index
}

#[inline(never)]
pub fn early_exit(mut index: u64, count: u8, stop: u8, history: &mut [u64; 32]) -> u64 {
    let mut iteration = 0u8;
    let mut sum = 0u64;
    while iteration < count {
        if iteration == stop {
            break;
        }
        let product = index.wrapping_mul(23);
        history[usize::from(iteration)] = product;
        sum = sum.wrapping_add(product);
        index = index.wrapping_sub(0x1000_0001);
        iteration = iteration.wrapping_add(1);
    }
    sum ^ index ^ u64::from(iteration)
}
