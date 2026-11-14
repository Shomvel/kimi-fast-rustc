//! RS5 borrow-permission fixtures (P5.x positive, N5.x negative). Every
//! program is defined under Stacked and Tree Borrows; the driver checks the
//! Rust-level answers and the three arms must agree byte for byte.

use core::cell::{Cell, RefCell};

/// P5.1: the second read of a Frozen `&u32` reuses the first across `f`.
#[inline(never)]
pub fn frozen_reload(x: &u32, f: fn()) -> u32 {
    let a = *x;
    f();
    a.wrapping_add(*x)
}

/// P5.2: slice element reuse across an unknown call.
#[inline(never)]
pub fn frozen_slice(s: &[u64], f: fn()) -> u64 {
    let a = s[0];
    f();
    a.wrapping_add(s[0])
}

/// A loop whose body calls `f`; the Frozen read before the loop serves
/// every iteration.
#[inline(never)]
pub fn frozen_loop(x: &u64, rounds: u32, f: fn()) -> u64 {
    let mut sum = *x;
    let mut round = 0;
    while round < rounds {
        f();
        sum = sum.wrapping_add(*x);
        round += 1;
    }
    sum
}

#[inline(never)]
fn observe(x: &u32) -> u32 {
    x.wrapping_mul(7)
}

/// P5.3: the local passed through a Frozen call-site argument keeps its value.
#[inline(never)]
pub fn observed_local(seed: u32) -> u32 {
    let x = seed.wrapping_mul(3);
    let seen = observe(&x);
    x.wrapping_add(seen)
}

/// The match-arm re-read shape of `semver::parse::identifier`.
#[inline(never)]
pub fn classify(bytes: &[u8], index: usize) -> u32 {
    match bytes.get(index) {
        Some(b'A'..=b'Z') | Some(b'a'..=b'z') | Some(b'-') => 1,
        Some(b'0'..=b'9') => 2,
        Some(_) => 3,
        None => 0,
    }
}

/// N5.1: `&Cell` is not Freeze; the callback's write is observed.
#[inline(never)]
pub fn cell_reload(c: &Cell<u32>, f: fn(&Cell<u32>)) -> u32 {
    let a = c.get();
    f(c);
    a.wrapping_add(c.get())
}

pub struct Mixed {
    pub n: u32,
    pub c: Cell<u32>,
}

/// N5.2: a struct with an `UnsafeCell` field carries no permission.
#[inline(never)]
pub fn mixed_reload(m: &Mixed, f: fn(&Mixed)) -> u32 {
    let a = m.c.get().wrapping_add(m.n);
    f(m);
    a.wrapping_add(m.c.get()).wrapping_add(m.n)
}

/// N5.3: a `RefCell` written by the callback.
#[inline(never)]
pub fn refcell_reload(c: &RefCell<u32>, f: fn(&RefCell<u32>)) -> u32 {
    let a = *c.borrow();
    f(c);
    a.wrapping_add(*c.borrow())
}

/// N5.5: a raw pointer derived from `&mut` writes the same place; the
/// reference then reads it. (Writing through `x` between the two raw writes
/// is defined only under Tree Borrows, so the fixture keeps the SB+TB form.)
#[inline(never)]
pub fn raw_from_unique(x: &mut u32) -> u32 {
    let p = x as *mut u32;
    unsafe {
        *p = 1;
        *p = (*p).wrapping_add(1);
    }
    *x
}

static mut CAPTURED: *mut u32 = core::ptr::null_mut();

#[inline(never)]
pub fn capture_unique(x: &mut u32) {
    unsafe { CAPTURED = x };
}

#[inline(never)]
pub fn write_captured(value: u32) {
    unsafe { *CAPTURED = value };
}

/// N5.7: an address captured by one call is written by a later call.
#[inline(never)]
pub fn captured_then_written(value: u32) -> u32 {
    let mut a = 1u32;
    capture_unique(&mut a);
    write_captured(value);
    a
}

pub trait Probe {
    fn read(&self) -> u32;
}

impl Probe for u32 {
    fn read(&self) -> u32 {
        *self
    }
}

impl Probe for Cell<u32> {
    fn read(&self) -> u32 {
        self.get()
    }
}

/// N5.12: one definition, Frozen for `u32` and None for `Cell<u32>`.
#[inline(never)]
pub fn generic_reload<T: Probe>(x: &T, f: fn(&T)) -> u32 {
    let a = x.read();
    f(x);
    a.wrapping_add(x.read())
}
