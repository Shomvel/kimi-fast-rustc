#![no_std]

#[inline(never)]
pub fn retyped_sibling(seed: u64, rounds: usize) -> u64 {
    let mut state = (seed, seed.rotate_left(9));
    for i in 0..rounds {
        state.0 = state.0.wrapping_add(i as u64).rotate_left(3);
        // The read stays within the initialized u64 field on either endianness.
        let low = unsafe { (&raw const state.1).cast::<u32>().read() };
        state.0 ^= u64::from(low);
        state.1 = state.1.wrapping_add(state.0);
    }
    state.0 ^ state.1
}

#[inline(never)]
pub fn copied_sibling(seed: u64, rounds: usize, output: &mut u64) -> u64 {
    let mut state = (seed, seed.rotate_left(11));
    for i in 0..rounds {
        state.0 = state.0.wrapping_add(i as u64).rotate_left(3);
        // The output belongs to the caller and is disjoint from this local.
        unsafe { core::ptr::copy_nonoverlapping(&raw const state.1, output, 1) };
        state.0 ^= *output;
        state.1 = state.1.wrapping_add(state.0);
    }
    state.0 ^ state.1
}

#[inline(never)]
pub fn nested_copy(seed: u64, rounds: usize, output: &mut u64) -> u64 {
    let mut state = ((seed, seed.rotate_left(7)), seed.rotate_left(19));
    for i in 0..rounds {
        state.0.0 = state.0.0.wrapping_add(i as u64).rotate_left(3);
        unsafe { core::ptr::copy_nonoverlapping(&raw const state.0.1, output, 1) };
        state.0.0 ^= *output;
        state.0.1 = state.0.1.wrapping_add(state.0.0);
        state.1 = state.1.wrapping_add(state.0.0);
    }
    state.0.0 ^ state.0.1 ^ state.1
}

#[inline(never)]
fn array_copy<const N: usize>(seed: u64, rounds: usize, output: &mut u64) -> u64 {
    let mut state = [seed; N];
    for i in 0..rounds {
        state[0] = state[0].wrapping_add(i as u64).rotate_left(3);
        unsafe { core::ptr::copy_nonoverlapping(&raw const state[N - 1], output, 1) };
        state[0] ^= *output;
        state[N - 1] = state[N - 1].wrapping_add(state[0]);
    }
    state[0] ^ state[N - 1]
}

pub fn array_two(seed: u64, rounds: usize, output: &mut u64) -> u64 {
    array_copy::<2>(seed, rounds, output)
}

pub fn array_five(seed: u64, rounds: usize, output: &mut u64) -> u64 {
    array_copy::<5>(seed, rounds, output)
}

pub fn array_eleven(seed: u64, rounds: usize, output: &mut u64) -> u64 {
    array_copy::<11>(seed, rounds, output)
}
