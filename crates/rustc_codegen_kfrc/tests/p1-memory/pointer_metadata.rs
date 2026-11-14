//! Metadata of a sized pointee is unit and never dereferences its pointer.

#[inline(never)]
fn callback(value: u64) -> u64 {
    value.wrapping_add(3)
}

#[inline(never)]
pub fn thin(seed: u64) -> u64 {
    let mut value = seed;
    let shared = std::ptr::addr_of!(value);
    let mutable = std::ptr::addr_of_mut!(value);
    let function: fn(u64) -> u64 = callback;
    std::hint::black_box(std::ptr::metadata(shared));
    std::hint::black_box(std::ptr::metadata(mutable));
    std::hint::black_box(std::ptr::metadata(&value));
    std::hint::black_box(std::ptr::metadata(&function));
    std::hint::black_box(std::ptr::metadata(std::ptr::null::<u64>()));
    std::hint::black_box(std::ptr::metadata(1_usize as *const [u64; 3]));
    function(std::hint::black_box(value))
}
