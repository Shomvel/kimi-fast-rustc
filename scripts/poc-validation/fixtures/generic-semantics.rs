//! True definition-R specialization acceptance: one generic body, multiple
//! concrete substitutions, and a layout question that makes a trait call dead.
use std::cmp::Ordering;
use std::hint::black_box;

#[inline(never)]
fn greater<T: PartialOrd>(a: &T, b: &T) -> bool {
    a > b
}

#[derive(PartialEq)]
struct Reverse(u32);
impl PartialOrd for Reverse {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        other.0.partial_cmp(&self.0)
    }
    fn gt(&self, other: &Self) -> bool {
        self.0 < other.0
    }
}

trait Label {
    fn value() -> usize;
}
impl Label for () {
    #[inline(never)]
    fn value() -> usize {
        // This target must not be resolved by choose::<()> after SizeAlign=0.
        99
    }
}
impl Label for u32 {
    #[inline(never)]
    fn value() -> usize {
        23
    }
}
#[inline(never)]
fn choose<T: Label>() -> usize {
    if std::mem::size_of::<T>() == 0 {
        7
    } else {
        T::value()
    }
}
#[inline(never)]
fn drop_question<T>() -> usize {
    if std::mem::needs_drop::<T>() { 13 } else { 11 }
}
#[inline(never)]
fn unwrap_or(value: Option<&u32>, fallback: u32) -> u32 {
    match value {
        Some(value) => *value,
        None => fallback,
    }
}
fn main() {
    let high = black_box(u32::MAX);
    let low = black_box(0u32);
    println!(
        "greater unsigned={} signed={} custom={}",
        greater(&high, &low),
        greater(&(high as i32), &(low as i32)),
        greater(&Reverse(high), &Reverse(low))
    );
    println!("choose zst={} sized={}", choose::<()>(), choose::<u32>());
    println!(
        "drop scalar={} string={}",
        drop_question::<u32>(),
        drop_question::<String>()
    );
    println!(
        "variant some={} none={}",
        unwrap_or(Some(&high), 31),
        unwrap_or(None, 31)
    );
}
