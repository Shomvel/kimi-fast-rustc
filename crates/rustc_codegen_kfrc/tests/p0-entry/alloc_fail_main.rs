//! Invoke the allocation-failure handler deterministically, without exhausting
//! the sandbox's memory. This exercises the generated error-handler wrapper.

fn main() {
    let layout = unsafe { std::alloc::Layout::from_size_align_unchecked(16, 8) };
    std::alloc::handle_alloc_error(layout);
}
