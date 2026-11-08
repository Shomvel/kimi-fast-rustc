// W05：&mut Vec<u8> 循环 push 另一个 &[u8]。
use std::hint::black_box;

#[inline(never)]
pub fn append(dst: &mut Vec<u8>, src: &[u8]) {
    for &b in src {
        dst.push(b)
    }
}

#[inline(never)]
pub fn append_ext(dst: &mut Vec<u8>, src: &[u8]) {
    dst.extend_from_slice(src)
}

fn main() {
    let src: Vec<u8> = (0..=255u8).collect();
    let mut a = Vec::new();
    let mut b = Vec::with_capacity(4);
    for _ in 0..black_box(3) {
        append(&mut a, black_box(&src));
        append_ext(&mut b, black_box(&src));
    }
    assert_eq!(a, b);
    println!("{} {}", a.len(), a[300]);
}
