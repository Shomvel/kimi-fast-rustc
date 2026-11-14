//! Logical zero-sized fields still control drop and closure captures.

static mut DROPS: u64 = 0;

struct Fragment;

impl Drop for Fragment {
    fn drop(&mut self) {
        unsafe {
            let counter = std::ptr::addr_of_mut!(DROPS);
            counter.write(counter.read().wrapping_add(1));
        }
    }
}

#[inline(never)]
pub fn aggregate_drops() -> u64 {
    unsafe { std::ptr::addr_of_mut!(DROPS).write(0) };
    {
        let tuple = ((), Fragment);
        std::hint::black_box(&tuple);
    }
    {
        let parts = (Fragment, ((), Fragment));
        let callback = move || drop(parts);
        callback();
    }
    {
        let array = [Fragment, Fragment, Fragment];
        std::hint::black_box(&array);
    }
    unsafe { std::ptr::addr_of!(DROPS).read() }
}
