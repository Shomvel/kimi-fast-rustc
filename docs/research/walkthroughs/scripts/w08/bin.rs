// W08 下游 crate：调用上游的各类函数。
use w08lib::*;

fn main() {
    let a: u32 = std::env::args().nth(1).map_or(7, |s| s.parse().unwrap());
    let p = Point::new(a, 4);
    println!("{} {} {}", generic_sum(&[1u8, 2, 3]), scale(a), plain(a));
    println!("{} {}", checked_div(100, a), norm1(&p));
    println!("{} {}", Shape::area(&200), line_here());
}
