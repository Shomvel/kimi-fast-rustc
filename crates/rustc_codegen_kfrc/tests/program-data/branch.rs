//! Compiled twice with different crate disambiguators. Both libraries contain
//! generic instances, nested data relocations and cross-module references.
extern crate program_shared;

#[inline(never)]
fn hash(bytes: &[u8], mut value: u64) -> u64 {
    for &byte in bytes {
        value = value.wrapping_mul(131).wrapping_add(u64::from(byte));
    }
    value
}

#[inline(never)]
fn callback_a(value: u64) -> u64 {
    value.wrapping_mul(7).wrapping_add(3)
}

#[inline(never)]
fn callback_b(value: u64) -> u64 {
    value.rotate_left(17) ^ 0xa53b
}

static CALLBACKS: [fn(u64) -> u64; 2] = [callback_a, callback_b];

// Holding a slice of slices forces data -> data -> bytes relocations.
const WORDS: &[&[u8]] = &[b"crate-owned-constants", b"nested-relocations"];

mod first {
    #[inline(never)]
    pub fn run(value: u64) -> u64 {
        let text = b"identical-literal-used-in-two-codegen-units";
        let value = super::hash(text, value);
        program_shared::generic::<[u8; 7]>(value)
    }
}

mod second {
    #[inline(never)]
    pub fn run(value: u64) -> u64 {
        let text = b"identical-literal-used-in-two-codegen-units";
        let value = super::hash(text, value);
        program_shared::generic::<[u8; 7]>(value)
    }
}

trait Step {
    fn apply(&self, value: u64) -> u64;
}
struct Add(u64);
impl Step for Add {
    #[inline(never)]
    fn apply(&self, value: u64) -> u64 {
        value.wrapping_add(self.0)
    }
}

#[inline(never)]
fn virtual_call(step: &dyn Step, value: u64) -> u64 {
    step.apply(value)
}

#[inline(never)]
pub fn evaluate(value: u64) -> u64 {
    let first = first::run(value);
    let second = second::run(value.wrapping_add(1));
    let word = WORDS[(value & 1) as usize];
    let result = hash(word, first ^ second);
    virtual_call(&Add(13), CALLBACKS[(value & 1) as usize](result))
}

#[inline(never)]
pub fn exchange(value: u64) -> u64 {
    program_shared::exchange(value)
}

#[inline(never)]
pub fn generic_address() -> fn(u64) -> u64 {
    program_shared::generic::<[u8; 7]>
}

#[inline(never)]
#[cfg(program_tls)]
pub fn tls_read() -> u64 {
    program_shared::tls_read()
}

#[inline(never)]
#[cfg(program_tls)]
pub fn tls_write(value: u64) {
    program_shared::tls_write(value)
}
