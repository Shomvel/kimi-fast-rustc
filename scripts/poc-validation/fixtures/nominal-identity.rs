//! Same short names inside nested generic arguments must retain nominal identity.
use std::hint::black_box;

mod left {
    #[repr(C)]
    pub struct Token(pub u32);
    pub fn item() -> u32 {
        17
    }
}
mod right {
    #[repr(C)]
    pub struct Token(pub u32, pub u32);
    pub fn item() -> u32 {
        29
    }
}
#[repr(C)]
struct Envelope<T>(T);
trait Score {
    fn score(&self) -> u32;
}
impl Score for left::Token {
    #[inline(never)]
    fn score(&self) -> u32 {
        self.0
    }
}
impl Score for right::Token {
    #[inline(never)]
    fn score(&self) -> u32 {
        self.0 ^ self.1
    }
}
#[inline(never)]
fn nested<T: Score>(value: &Envelope<T>) -> u32 {
    value.0.score()
}
#[inline(never)]
fn function_item<F: Fn() -> u32>(call: F) -> u32 {
    call()
}
fn main() {
    let left = Envelope(left::Token(black_box(13)));
    let right = Envelope(right::Token(black_box(19), black_box(7)));
    println!("nested {} {}", nested(&left), nested(&right));
    println!(
        "items {} {}",
        function_item(left::item),
        function_item(right::item)
    );
    println!("closures {} {}", function_item(|| 31), function_item(|| 43));
}
