use generic_memory::*;

#[unsafe(no_mangle)]
pub extern "Rust" fn generic_memory_observe_pair(value: &(u64, u64)) -> u64 {
    value.0.wrapping_add(value.1)
}

#[unsafe(no_mangle)]
pub extern "Rust" fn generic_memory_observe_word(value: &u64) -> u64 {
    *value
}

#[unsafe(no_mangle)]
pub extern "Rust" fn generic_memory_observe_nested(value: &(u64, (u64, u64))) -> u64 {
    value.0.wrapping_add(value.1 .0.rotate_left(13)) ^ value.1 .1
}

#[unsafe(no_mangle)]
pub extern "Rust" fn generic_memory_mutate_cell(
    value: &std::cell::UnsafeCell<u64>,
    next: u64,
) -> u64 {
    // UnsafeCell permits this mutation through a shared reference. The caller
    // must retain a fresh read after this call because the pointee is !Freeze.
    unsafe {
        let old = *value.get();
        *value.get() = next;
        old
    }
}

/// # Safety
/// `sibling` is an aligned, writable u64 disjoint from `protected`.
#[unsafe(no_mangle)]
pub unsafe extern "Rust" fn generic_memory_observe_and_write(
    protected: &u64,
    sibling: *mut u64,
    next: u64,
) -> u64 {
    let observed = *protected;
    unsafe {
        *sibling = next;
    }
    observed
}

fn main() {
    let mut state = 0x9173_cafe_0a7d_b5e1u64;
    let mut checksum = 0u64;
    for i in 0..20_006 {
        let a = match i {
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
        let mut pair = (!a, !b);
        let value = overwritten_pair(&mut pair, a, b);
        assert_eq!(pair, (a.wrapping_add(b), b));
        assert_eq!(value, a ^ a.wrapping_add(b) ^ b);
        checksum = checksum.wrapping_add(value);

        let mut fields = [!a, !b, 17, 29];
        let value = forward_disjoint_fields(&mut fields, a, b);
        assert_eq!(fields, [a, b, a.wrapping_add(a), a ^ b]);
        assert_eq!(value, b ^ a.wrapping_add(a) ^ (a ^ b));
        checksum ^= value;

        for alias in [false, true] {
            let mut words = [!a, !b];
            let pointer = words.as_mut_ptr();
            // Both pointers are aligned and writable; aliasing raw pointers
            // are intentional and no Rust reference is alive during the call.
            let actual = unsafe { raw_may_alias(pointer, pointer.add(usize::from(!alias)), a, b) };
            let second = if alias { b } else { a };
            let final_q = if alias { a ^ b } else { b };
            assert_eq!(actual, (a, second, final_q));
            assert_eq!(words, [a ^ second, if alias { !b } else { b }]);
            checksum = checksum.wrapping_add(actual.0 ^ actual.1 ^ actual.2);
        }

        let mut word = !a;
        // `word` is initialized, aligned and live for both calls.
        let repeated = unsafe { raw_repeated_overwrite(&mut word, a, b) };
        assert_eq!(repeated, a ^ b);
        assert_eq!(word, a ^ b);
        let byte = (b >> 37) as u8;
        let changed = (a & !(0xff << 24)) | (u64::from(byte) << 24);
        let difference = unsafe { overlapping_byte(&mut word, a, byte) };
        assert_eq!(word, changed);
        assert_eq!(difference, a ^ changed);
        checksum ^= repeated ^ difference;

        let mut pair = (!a, !b);
        assert_eq!(
            observed_before_overwrite(&mut pair, a, b),
            a.wrapping_add(b)
        );
        assert_eq!(pair, (a ^ b, b));
        for condition in [false, true] {
            let mut pair = (!a, !b);
            let expected = if condition { b } else { a };
            assert_eq!(conditional_overwrite(&mut pair, condition, a, b), expected);
            assert_eq!(pair, (expected, expected ^ a));
            checksum = checksum.wrapping_add(pair.0 ^ pair.1);

            let mut pair = (!a, !b);
            let expected = if condition { b.wrapping_add(1) } else { a ^ b };
            assert_eq!(
                both_branches_overwrite(&mut pair, condition, a, b),
                expected
            );
            assert_eq!(pair, (expected, b ^ expected));
            checksum ^= pair.0.wrapping_add(pair.1);

            let mut pair = (!a, !b);
            let expected = if condition { a.wrapping_add(b) } else { a ^ b };
            assert_eq!(
                observed_branch_overwrite(&mut pair, condition, a, b),
                expected
            );
            assert_eq!(pair, (if condition { b } else { a.wrapping_add(b) }, b));
            checksum = checksum.wrapping_add(expected);
        }

        let mut pair = (!a, !b);
        assert_eq!(
            frozen_whole_object_reload(&mut pair, a, b),
            (a.wrapping_add(b), b)
        );
        assert_eq!(pair, (a, b));
        assert_eq!(frozen_exact_field_reload(&mut pair, a ^ b), (a ^ b, a ^ b));
        assert_eq!(pair, (a, a ^ b));
        // The raw pointer is valid for the complete pair and all shared
        // reborrows end before the driver checks the external memory again.
        assert_eq!(
            unsafe { frozen_raw_reborrow_reload(&mut pair, b, a) },
            (b.wrapping_add(a), a)
        );
        assert_eq!(pair, (b, a));

        let mut nested = (!a, (!b, 19));
        let observed = (a ^ b).wrapping_add(a.rotate_left(13)) ^ b;
        assert_eq!(frozen_nested_reload(&mut nested, a, b), (observed, a, b));
        assert_eq!(nested, (a ^ b, (a, b)));
        let cell = std::cell::UnsafeCell::new(a);
        assert_eq!(unsafe_cell_reload(&cell, b), (a, a, b));
        assert_eq!(unsafe { *cell.get() }, b);

        let changed = a.rotate_left(7) ^ b;
        assert_eq!(
            unsafe { protected_sibling_mutation(&mut pair, a, b, changed) },
            (a, a, changed)
        );
        assert_eq!(pair, (a, changed));
        checksum = checksum.wrapping_add(observed ^ pair.0 ^ pair.1 ^ b);
    }
    println!("generic memory: 20006 cases passed; checksum={checksum:016x}");
}
