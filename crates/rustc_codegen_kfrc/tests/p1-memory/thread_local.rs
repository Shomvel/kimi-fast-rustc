//! Constant and lazy TLS are persistent within a thread and isolated across it.

use std::cell::{Cell, RefCell, UnsafeCell};
use std::collections::HashMap;

#[thread_local]
static INITIALIZED: u64 = 17;
#[thread_local]
static mut NONZERO: u64 = 23;
#[thread_local]
static mut ZERO: u64 = 0;
#[thread_local]
static INTERIOR: UnsafeCell<[u64; 2]> = UnsafeCell::new([29, 31]);

#[inline(never)]
pub fn immutable_value() -> u64 {
    unsafe { std::ptr::addr_of!(INITIALIZED).read() }
}

#[inline(never)]
pub fn read() -> (u64, u64, u64, u64, u64, u64) {
    unsafe {
        let cell = INTERIOR.get().cast::<u64>();
        (
            std::ptr::addr_of!(NONZERO).read(),
            std::ptr::addr_of!(ZERO).read(),
            cell.read(),
            cell.add(1).read(),
            CONST_WORD.with(Cell::get),
            LAZY_WORD.with(Cell::get),
        )
    }
}

#[inline(never)]
pub fn write(seed: u64) {
    unsafe {
        std::ptr::addr_of_mut!(NONZERO).write(seed.wrapping_add(5));
        std::ptr::addr_of_mut!(ZERO).write(seed.wrapping_mul(3));
        let cell = INTERIOR.get().cast::<u64>();
        cell.write(seed.wrapping_add(7));
        cell.add(1).write(seed.wrapping_mul(11));
    }
    CONST_WORD.with(|cell| cell.set(seed.wrapping_add(13)));
    LAZY_WORD.with(|cell| cell.set(seed.wrapping_mul(17)));
}

struct DropToken {
    marker: *mut u64,
    value: u64,
}

impl Drop for DropToken {
    fn drop(&mut self) {
        unsafe {
            self.marker
                .write(self.marker.read().wrapping_add(self.value))
        };
    }
}

std::thread_local! {
    static CONST_WORD: Cell<u64> = const { Cell::new(37) };
    static LAZY_WORD: Cell<u64> = Cell::new(lazy_seed());
    static DROP_TOKEN: RefCell<Option<DropToken>> = const { RefCell::new(None) };
    static LAZY_DROP_TOKEN: RefCell<Option<DropToken>> = RefCell::new(lazy_empty());
}

#[inline(never)]
fn lazy_seed() -> u64 {
    immutable_value().wrapping_add(24)
}

#[inline(never)]
fn lazy_empty() -> Option<DropToken> {
    None
}

/// The caller keeps two marker words alive and reads them after this thread exits.
#[inline(never)]
pub unsafe fn set_exit_marker(marker: *mut u64, value: u64) {
    DROP_TOKEN.with(|slot| {
        *slot.borrow_mut() = Some(DropToken { marker, value });
    });
    LAZY_DROP_TOKEN.with(|slot| {
        *slot.borrow_mut() = Some(DropToken {
            marker: unsafe { marker.add(1) },
            value: value.wrapping_mul(10),
        });
    });
}

#[inline(never)]
pub fn lazy_random_state(seed: u64) -> u64 {
    let mut values = HashMap::new();
    values.insert(seed, seed.wrapping_mul(3));
    values.insert(seed.wrapping_add(1), seed.wrapping_add(19));
    values[&seed]
        .wrapping_add(values[&seed.wrapping_add(1)])
        .wrapping_add(values.len() as u64)
}
