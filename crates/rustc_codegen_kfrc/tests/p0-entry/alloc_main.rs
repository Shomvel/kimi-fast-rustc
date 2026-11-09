//! Exercise the generated allocator wrappers through real Vec allocation,
//! repeated growth/reallocation, initialized storage, and destructor cleanup.

unsafe extern "C" {
    fn puts(message: *const i8) -> i32;
    fn abort() -> !;
}

fn main() {
    {
        let mut values = Vec::<u64>::with_capacity(1);
        for index in 0..65_u64 {
            values.push(index.wrapping_mul(17).wrapping_add(3));
        }
        if values.len() != 65 || values.capacity() < 65 {
            unsafe { abort() };
        }
        let mut sum = 0_u64;
        for index in 0..65_usize {
            let expected = (index as u64).wrapping_mul(17).wrapping_add(3);
            if values[index] != expected {
                unsafe { abort() };
            }
            sum = sum.wrapping_add(values[index]);
        }
        if sum != 35_555 {
            unsafe { abort() };
        }
        // Leaving the scope runs Vec's real drop glue and allocator dealloc.
    }
    unsafe {
        puts(b"kfrc allocator Vec growth/drop passed\0".as_ptr().cast());
    }
}
