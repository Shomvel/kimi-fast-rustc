#![feature(thread_local)]
#[thread_local]
static VALUE: u64 = 7;
#[inline(never)]
pub fn tls_address() -> *const u64 {
    &VALUE
}
