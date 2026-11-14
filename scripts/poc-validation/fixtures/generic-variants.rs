//! Generic enum facts survive definition-level promotion and normalization.
use std::hint::black_box;

#[derive(Clone, Copy)]
enum Choice<T> {
    Empty,
    Full(T),
}
#[inline(never)]
fn select<T: Copy>(value: Choice<T>, fallback: T) -> T {
    match value {
        Choice::Full(value) => value,
        Choice::Empty => fallback,
    }
}
#[inline(never)]
fn nested<T: Copy>(value: Option<Option<T>>, fallback: T) -> T {
    match value {
        Some(Some(value)) => value,
        Some(None) | None => fallback,
    }
}
#[inline(never)]
fn known<T: Copy>(value: T, fallback: T) -> T {
    let chosen = Choice::Full(value);
    match chosen {
        Choice::Full(value) => value,
        Choice::Empty => fallback,
    }
}
fn main() {
    let high = black_box(0x8000_0001u32);
    let low = black_box(-7i32);
    println!(
        "select unsigned {} {}",
        select(Choice::Full(high), 13),
        select(Choice::Empty::<u32>, 13)
    );
    println!(
        "select signed {} {}",
        select(Choice::Full(low), 17),
        select(Choice::Empty::<i32>, 17)
    );
    println!(
        "nested {} {} {}",
        nested(Some(Some(high)), 19),
        nested(Some(None::<u32>), 19),
        nested(None::<Option<u32>>, 19)
    );
    println!("known {} {}", known(high, 23), known(low, 29));
}
