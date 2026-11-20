use std::sync::atomic::*;

macro_rules! integer {
    ($function:ident, $atomic:ty, $value:ty) => {
        #[unsafe(no_mangle)]
        pub fn $function(
            cell: &$atomic,
            value: $value,
            delta: $value,
        ) -> (
            $value,
            $value,
            $value,
            $value,
            $value,
            Result<$value, $value>,
            Result<$value, $value>,
            $value,
        ) {
            cell.store(value, Ordering::Relaxed);
            let before = cell.load(Ordering::Acquire);
            let old = cell.fetch_sub(delta, Ordering::AcqRel);
            let after = cell.load(Ordering::SeqCst);
            let add_old = cell.fetch_add(delta, Ordering::Relaxed);
            let swapped = cell.swap(after, Ordering::AcqRel);
            // Failed CAS returns the observed value and does not write.
            let failed = cell.compare_exchange(value, delta, Ordering::AcqRel, Ordering::Acquire);
            let succeeded = cell.compare_exchange(after, value, Ordering::SeqCst, Ordering::SeqCst);
            loop {
                if cell
                    .compare_exchange_weak(value, after, Ordering::Release, Ordering::Relaxed)
                    .is_ok()
                {
                    break;
                }
            }
            (
                before,
                old,
                after,
                add_old,
                swapped,
                failed,
                succeeded,
                cell.load(Ordering::SeqCst),
            )
        }
    };
}
integer!(unsigned8, AtomicU8, u8);
integer!(unsigned16, AtomicU16, u16);
integer!(unsigned32, AtomicU32, u32);
integer!(unsigned64, AtomicU64, u64);
integer!(signed8, AtomicI8, i8);
integer!(signed16, AtomicI16, i16);
integer!(signed32, AtomicI32, i32);
integer!(signed64, AtomicI64, i64);

#[unsafe(no_mangle)]
pub fn pointer(
    cell: &AtomicPtr<u64>,
    value: *mut u64,
) -> (*mut u64, *mut u64, Result<*mut u64, *mut u64>) {
    cell.store(value, Ordering::Release);
    let swapped = cell.swap(value, Ordering::SeqCst);
    let compared = cell.compare_exchange(value, value, Ordering::AcqRel, Ordering::Acquire);
    (cell.load(Ordering::Acquire), swapped, compared)
}

#[unsafe(no_mangle)]
pub fn decrement(cell: &AtomicUsize) -> usize {
    cell.fetch_sub(1, Ordering::Relaxed)
}

#[unsafe(no_mangle)]
pub fn decrement_cas(cell: &AtomicUsize) -> usize {
    let mut old = cell.load(Ordering::Relaxed);
    loop {
        match cell.compare_exchange_weak(
            old,
            old.wrapping_sub(1),
            Ordering::AcqRel,
            Ordering::Relaxed,
        ) {
            Ok(value) => return value,
            Err(value) => old = value,
        }
    }
}

#[unsafe(no_mangle)]
pub fn publish(data: *mut usize, flag: &AtomicUsize, value: usize) {
    unsafe {
        *data = value;
    }
    flag.store(1, Ordering::Release);
}

#[unsafe(no_mangle)]
pub fn receive(data: *const usize, flag: &AtomicUsize) -> usize {
    while flag.load(Ordering::Acquire) == 0 {
        std::hint::spin_loop();
    }
    unsafe { *data }
}

#[unsafe(no_mangle)]
pub fn store_then_load(own: &AtomicUsize, other: &AtomicUsize) -> usize {
    own.store(1, Ordering::SeqCst);
    other.load(Ordering::SeqCst)
}

#[unsafe(no_mangle)]
pub fn fences() {
    fence(Ordering::Acquire);
    fence(Ordering::Release);
    fence(Ordering::AcqRel);
    fence(Ordering::SeqCst);
    compiler_fence(Ordering::SeqCst);
}
