// W04: drop flags, `?` early return, may-unwind vs no-unwind calls.
use std::num::ParseIntError;

#[inline(never)]
fn may_panic(v: &[u8], i: usize) -> u8 { v[i] } // bounds check: may unwind
#[inline(never)]
fn no_panic(x: usize) -> usize { x.wrapping_mul(31) ^ 7 } // leaf, cannot unwind
#[inline(never)]
fn even(n: usize) -> bool { if n == 0 { true } else { odd(n - 1) } } // SCC {even, odd}
#[inline(never)]
fn odd(n: usize) -> bool { if n == 0 { false } else { even(n - 1) } }
#[inline(never)]
fn consume(s: String) -> usize { s.len() }

#[inline(never)]
pub fn work(t: &str, flag: bool) -> Result<usize, ParseIntError> {
    let name = String::from(t);
    let mut buf: Vec<u8> = Vec::with_capacity(8);
    buf.extend_from_slice(name.as_bytes());
    let n: usize = t.trim().parse()?; // early return drops buf, name
    let mut acc = no_panic(n) + even(n) as usize;
    if flag { acc += consume(name); } // conditional move -> drop flag
    acc += may_panic(&buf, n) as usize; // unwind here must test the flag
    Ok(acc)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    println!("{:?}", work(a.get(1).map_or("0", |s| s), a.len() > 2));
}
