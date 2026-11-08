// W06: one generic body, many T; Vec<T>::push for several element types.
use std::hint::black_box as bb;

#[inline(never)]
pub fn largest<T: PartialOrd + Copy>(xs: &[T]) -> T {
    let mut m = xs[0];
    for &x in xs {
        if x > m { m = x; }
    }
    m
}

#[inline(never)]
pub fn push_one<T>(v: &mut Vec<T>, x: T) { v.push(x) }

fn main() {
    println!("{}", largest(bb(&[3u32, 9, 4][..])));
    println!("{}", largest(bb(&[-3i32, 9, 4][..])));
    println!("{}", largest(bb(&['a', 'z', 'q'][..])));
    println!("{}", largest(bb(&[3u64, 9, 4][..])));
    println!("{}", largest(bb(&[3.0f64, f64::NAN, 4.0][..])));
    let (mut a, mut b, mut c, mut d) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    push_one(&mut a, bb(1u32));
    push_one(&mut b, bb(-1i32));
    push_one(&mut c, bb([1u8; 4]));
    push_one(&mut d, bb('x'));
    println!("{} {} {} {}", a.len(), b.len(), c.len(), d.len());
}
