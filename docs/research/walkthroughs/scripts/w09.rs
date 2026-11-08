// W09: large values — sret return, Box::new, big enum move + match, size thresholds.
use std::hint::black_box;
pub struct Big { pub a: [u64; 16], pub n: u32 } // 136 bytes, BackendRepr::Memory
pub enum Msg { Data([u8; 256]), Ping(u32), Quit }
#[inline(never)] pub fn make_big(x: u64) -> Big { Big { a: [x; 16], n: 7 } }
#[inline(never)] pub fn ret_big(x: u64) -> Big { let mut b = make_big(x); b.n += 1; b }
#[inline(never)] pub fn box_call(x: u64) -> Box<Big> { Box::new(make_big(x)) }
#[inline(never)] pub fn box_lit(x: u64) -> Box<Big> { Box::new(Big { a: [x; 16], n: 1 }) }
#[inline(never)] pub fn box_zero() -> Box<[u8; 4096]> { Box::new([0u8; 4096]) }
#[inline(never)] pub fn handle(m: Msg) -> u32 {
    match m { Msg::Data(d) => d[0] as u32 + d[255] as u32, Msg::Ping(p) => p, Msg::Quit => 0 }
}
#[inline(never)] pub fn route(x: u8) -> u32 {
    let m = if x > 9 { Msg::Data([x; 256]) } else if x > 0 { Msg::Ping(x as u32) } else { Msg::Quit };
    let m2 = m; handle(m2)
}
fn xor(a: &[u8]) -> u8 { a.iter().fold(0, |s, &b| s ^ b) }
#[inline(never)] pub fn make<const N: usize>(x: u8) -> [u8; N] { [x; N] }
#[inline(never)] pub fn take<const N: usize>(a: [u8; N]) -> u8 { xor(&a) }
#[inline(never)] pub fn take_opt<const N: usize>(o: Option<[u8; N]>) -> u8 { o.map_or(0, |a| xor(&a)) }
#[inline(never)] pub fn pipe<const N: usize>(x: u8) -> u8 {
    let a = make::<N>(x); take(a) ^ take(a) ^ take_opt(Some(make::<N>(x)))
}
#[inline(never)] pub fn boxed<const N: usize>(x: u8) -> Box<[u8; N]> { Box::new(make::<N>(x)) }
macro_rules! sizes { ($($n:literal)*) => { $( print!("{}:{}/{} ", $n, pipe::<$n>(black_box(3)), boxed::<$n>(black_box(3))[$n - 1]); )* } }
fn main() {
    let x = black_box(5u64);
    println!("{} {} {} {} {}", ret_big(x).n, box_call(x).a[15], box_lit(x).n, box_zero()[4095], route(black_box(12)));
    sizes!(4 8 16 24 32 64 128 256 4096); println!();
}
