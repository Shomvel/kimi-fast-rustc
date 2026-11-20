use std::cell::UnsafeCell;
use std::sync::{Barrier, atomic::*};
use std::thread;

fn main() {
    macro_rules! check {
        ($function:ident, $atomic:ty, $value:expr, $delta:expr) => {{
            let value = $value;
            let delta = $delta;
            let cell = <$atomic>::new(0);
            assert_eq!(
                atomics::$function(&cell, value, delta),
                (
                    value,
                    value,
                    value.wrapping_sub(delta),
                    value.wrapping_sub(delta),
                    value,
                    Err(value.wrapping_sub(delta)),
                    Ok(value.wrapping_sub(delta)),
                    value.wrapping_sub(delta)
                )
            );
        }};
    }
    check!(unsigned8, AtomicU8, 0u8, 1u8);
    check!(unsigned16, AtomicU16, 0u16, 1u16);
    check!(unsigned32, AtomicU32, 0u32, 1u32);
    check!(unsigned64, AtomicU64, 0u64, 1u64);
    check!(signed8, AtomicI8, i8::MIN, 1i8);
    check!(signed16, AtomicI16, i16::MIN, 1i16);
    check!(signed32, AtomicI32, i32::MIN, 1i32);
    check!(signed64, AtomicI64, i64::MIN, 1i64);
    let mut value = 123u64;
    let address = &mut value as *mut u64;
    let pointer = AtomicPtr::new(std::ptr::null_mut());
    assert_eq!(
        atomics::pointer(&pointer, address),
        (address, address, Ok(address))
    );
    atomics::fences();

    for decrement in [atomics::decrement, atomics::decrement_cas] {
        let counter = AtomicUsize::new(80_000);
        let sums: Vec<_> = thread::scope(|scope| {
            let threads: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| (0..10_000).map(|_| decrement(&counter) as u64).sum::<u64>())
                })
                .collect();
            threads
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect()
        });
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        assert_eq!(sums.iter().sum::<u64>(), 80_000 * 80_001 / 2);
    }

    struct Message(UnsafeCell<usize>);
    unsafe impl Sync for Message {}
    impl Message {
        fn get(&self) -> *mut usize {
            self.0.get()
        }
    }
    for iteration in 1..=200 {
        let message = Message(UnsafeCell::new(0));
        let flag = AtomicUsize::new(0);
        thread::scope(|scope| {
            scope.spawn(|| atomics::publish(message.get(), &flag, iteration));
            scope.spawn(|| assert_eq!(atomics::receive(message.get(), &flag), iteration));
        });
    }

    let a = AtomicUsize::new(0);
    let b = AtomicUsize::new(0);
    let result_a = AtomicUsize::new(0);
    let result_b = AtomicUsize::new(0);
    let barrier = Barrier::new(3);
    thread::scope(|scope| {
        scope.spawn(|| {
            for _ in 0..2_000 {
                barrier.wait();
                result_a.store(atomics::store_then_load(&a, &b), Ordering::Relaxed);
                barrier.wait();
            }
        });
        scope.spawn(|| {
            for _ in 0..2_000 {
                barrier.wait();
                result_b.store(atomics::store_then_load(&b, &a), Ordering::Relaxed);
                barrier.wait();
            }
        });
        for _ in 0..2_000 {
            a.store(0, Ordering::Relaxed);
            b.store(0, Ordering::Relaxed);
            barrier.wait();
            barrier.wait();
            assert!(result_a.load(Ordering::Relaxed) != 0 || result_b.load(Ordering::Relaxed) != 0);
        }
    });
    println!(
        "PASS: integer widths/signs, wrapping results, pointers, fences, 80000 concurrent RMW and CAS each, 200 publications, 2000 seqcst rounds"
    );
}
