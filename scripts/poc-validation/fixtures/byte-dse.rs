#![crate_type = "rlib"]

#[inline(never)]
pub fn halves(out: &mut u64, old: u64, next: u64) -> u64 {
    *out = old;
    let pointer = out as *mut u64 as *mut u32;
    // Both aligned half-word accesses are inside the borrowed u64. No shared
    // reference or typed read is live while its bytes are being overwritten.
    unsafe {
        *pointer = next as u32;
        *pointer.add(1) = (next >> 32) as u32;
    }
    *out
}

#[inline(never)]
pub fn bytes(out: &mut u64, old: u64, next: u64) -> u64 {
    *out = old;
    let pointer = out as *mut u64 as *mut u8;
    // Every byte access stays within the borrowed word. Deliberately use a
    // nonsequential order to exercise coverage rather than one growing prefix.
    unsafe {
        *pointer.add(3) = (next >> 24) as u8;
        *pointer.add(7) = (next >> 56) as u8;
        *pointer.add(1) = (next >> 8) as u8;
        *pointer.add(5) = (next >> 40) as u8;
        *pointer = next as u8;
        *pointer.add(6) = (next >> 48) as u8;
        *pointer.add(2) = (next >> 16) as u8;
        *pointer.add(4) = (next >> 32) as u8;
    }
    *out
}

#[inline(never)]
pub fn incomplete_branch(out: &mut u64, old: u64, next: u64, complete: bool) -> u64 {
    *out = old;
    let pointer = out as *mut u64 as *mut u32;
    // The first half is always written, and the second only on one arm.
    unsafe {
        *pointer = next as u32;
        if complete {
            *pointer.add(1) = (next >> 32) as u32;
        }
    }
    *out
}

unsafe extern "Rust" {
    fn byte_dse_observe(value: &u64) -> u64;
}

#[inline(never)]
pub fn observed_halfway(out: &mut u64, old: u64, next: u64) -> u64 {
    *out = old;
    let word = out as *mut u64;
    let pointer = word.cast::<u32>();
    // The observer's shared reborrow ends before the second write. Its input
    // is still a fully initialized word, containing the old high half.
    unsafe {
        *pointer = next as u32;
        let observed = byte_dse_observe(&*word);
        *pointer.add(1) = (next >> 32) as u32;
        observed ^ *word
    }
}
