//! Writable statics, immutable relocations, and interior-mutable storage.

use std::cell::UnsafeCell;

static VALUES: [u64; 4] = [11, 23, 37, 53];
static mut NONZERO: u64 = 17;
static mut ZERO: u64 = 0;
static mut POINTER: *const u64 = unsafe { VALUES.as_ptr().add(2) };

struct Shared(UnsafeCell<[u64; 2]>);
// This fixture accesses the cell from its single-threaded driver only.
unsafe impl Sync for Shared {}
static INTERIOR: Shared = Shared(UnsafeCell::new([29, 31]));

struct ReadOnlyPointer(*const u64);
// Its pointer targets immutable storage with process lifetime.
unsafe impl Sync for ReadOnlyPointer {}
static READ_ONLY_POINTER: ReadOnlyPointer = ReadOnlyPointer(unsafe { VALUES.as_ptr().add(1) });

#[inline(never)]
pub fn scalar_storage(seed: u64) -> u64 {
    unsafe {
        let nonzero = std::ptr::addr_of_mut!(NONZERO);
        let zero = std::ptr::addr_of_mut!(ZERO);
        let previous = nonzero.read().wrapping_add(zero.read());
        nonzero.write(seed.wrapping_add(7));
        zero.write(seed.wrapping_mul(3));
        previous ^ nonzero.read().wrapping_add(zero.read())
    }
}

#[inline(never)]
pub fn pointer_storage(seed: u64) -> u64 {
    unsafe {
        let pointer = std::ptr::addr_of_mut!(POINTER);
        let previous = pointer.read().read();
        pointer.write(VALUES.as_ptr().add((seed & 3) as usize));
        previous.wrapping_mul(5).wrapping_add(pointer.read().read())
    }
}

#[inline(never)]
pub fn interior_storage(seed: u64) -> u64 {
    unsafe {
        let pointer = INTERIOR.0.get().cast::<u64>();
        let previous = pointer.read().wrapping_add(pointer.add(1).read());
        pointer.write(seed.wrapping_add(13));
        pointer.add(1).write(seed.wrapping_mul(7));
        previous ^ pointer.read().wrapping_add(pointer.add(1).read())
    }
}

#[inline(never)]
pub fn immutable_storage(index: usize) -> u64 {
    let pointer_value = unsafe { READ_ONLY_POINTER.0.read() };
    VALUES[index & 3]
        .wrapping_mul(3)
        .wrapping_add(pointer_value)
}
