#![cfg_attr(program_tls, feature(thread_local))]
//! One dependency shared by two independently compiled, multi-CGU libraries.

static FIRST: [u8; 8] = *b"identity";
static SECOND: [u8; 8] = *b"identity";
static mut STATE: u64 = 19;
#[thread_local]
#[cfg(program_tls)]
static mut TLS: u64 = 37;

struct Link {
    next: &'static Link,
    value: u64,
}

static CYCLE_A: Link = Link {
    next: &CYCLE_B,
    value: 23,
};
static CYCLE_B: Link = Link {
    next: &CYCLE_A,
    value: 41,
};

// Each downstream crate instantiates this same definition. Its internal
// instance must remain linkable when promoted to hidden global visibility.
#[inline]
pub fn generic<T>(value: u64) -> u64 {
    value
        .wrapping_mul(0x9e3779b97f4a7c15)
        .rotate_left(std::mem::size_of::<T>() as u32)
        .wrapping_add(29)
}

#[inline(never)]
pub fn identities() -> bool {
    std::ptr::addr_of!(FIRST) != std::ptr::addr_of!(SECOND)
        && std::ptr::eq(CYCLE_A.next.next, &CYCLE_A)
        && CYCLE_A.value + CYCLE_A.next.value == 64
}

#[inline(never)]
pub fn exchange(value: u64) -> u64 {
    // The differential driver is the sole user of this process-wide state.
    unsafe {
        let address = std::ptr::addr_of_mut!(STATE);
        let old = address.read();
        address.write(value);
        old
    }
}

#[inline(never)]
#[cfg(program_tls)]
pub fn tls_read() -> u64 {
    unsafe { std::ptr::addr_of!(TLS).read() }
}

#[inline(never)]
#[cfg(program_tls)]
pub fn tls_write(value: u64) {
    unsafe { std::ptr::addr_of_mut!(TLS).write(value) }
}
