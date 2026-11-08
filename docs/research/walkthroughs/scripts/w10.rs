// W10: interior mutability vs. memory partitions (Cell, RefCell, atomics, partly-Freeze struct, !Unpin).
use std::cell::{Cell, RefCell};
use std::hint::black_box;
use std::marker::PhantomPinned;
use std::sync::atomic::{fence, AtomicU32, AtomicUsize, Ordering::*};
pub struct Mixed { pub n: u32, pub c: Cell<u32> } // only the Cell field is !Freeze
pub struct Pinned { pub n: u32, _p: PhantomPinned }
#[inline(never)] pub fn frozen(x: &u32, f: fn()) -> u32 { let a = *x; f(); a + *x }
#[inline(never)] pub fn cell(c: &Cell<u32>, f: fn()) -> u32 { let a = c.get(); f(); a + c.get() }
#[inline(never)] pub fn mixed(m: &Mixed, f: fn()) -> u32 { let a = m.n; f(); a + m.n }
#[inline(never)] pub fn mixed_nocall(m: &Mixed) -> u32 { let a = m.n; m.c.set(7); a + m.n }
#[inline(never)] pub fn shallow(r: &&Cell<u32>, f: fn()) -> u32 { let a = r.get(); f(); a + r.get() }
#[inline(never)] pub fn pair(p: (&u32, &Cell<u32>), f: fn()) -> u32 { let a = *p.0; f(); a + *p.0 + p.1.get() }
#[inline(never)] pub fn refcell(r: &RefCell<u32>) { *r.borrow_mut() += 1; }
#[inline(never)] pub fn unique_fence(x: &mut u32) { *x = 1; fence(Release); *x = 2; }
#[inline(never)] pub fn unique_acquire(x: &mut u32, a: &AtomicUsize) -> usize { *x += 1; let s = a.load(Acquire); *x as usize + s }
#[inline(never)] pub fn frozen_fence(x: &u32) -> u32 { let a = *x; fence(SeqCst); a + *x }
#[inline(never)] pub fn unique_atomic(x: &mut AtomicU32, f: fn()) -> u32 { x.store(1, Relaxed); f(); x.load(Relaxed) }
#[inline(never)] pub fn local_atomic() -> usize { let a = AtomicUsize::new(1); a.fetch_add(1, SeqCst); a.load(SeqCst) }
#[inline(never)] pub fn pinned(p: &mut Pinned, f: fn()) -> u32 { p.n = 1; f(); p.n }
fn noop() {}
pub fn main() {
    let f: fn() = black_box(noop);
    let (x, c, m, r, a) = (black_box(5u32), Cell::new(2), Mixed { n: 3, c: Cell::new(0) }, RefCell::new(1), AtomicUsize::new(1));
    let (mut u, mut au, mut p) = (0u32, AtomicU32::new(0), Pinned { n: 0, _p: PhantomPinned });
    let v = [frozen(&x, f), cell(&c, f), mixed(&m, f), mixed_nocall(&m), shallow(&&c, f), pair((&x, &c), f), frozen_fence(&x), unique_atomic(&mut au, f), pinned(&mut p, f)];
    refcell(&r); unique_fence(&mut u);
    let w = [unique_acquire(&mut u, &a), local_atomic()];
    assert_eq!((v, w, *r.borrow(), m.c.get(), u), ([10, 4, 6, 6, 4, 12, 10, 1, 1], [4, 2], 2, 7, 3));
}
