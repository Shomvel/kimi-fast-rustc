// W08 上游 crate：E1 改这行注释的文字。每类函数一个模块（rustc 增量下一模块一个 CGU）。
pub mod generic {
    pub fn generic_sum<T: Copy + Into<u64>>(xs: &[T]) -> u64 {
        xs.iter().map(|&x| x.into()).sum::<u64>() // E4
    }
}
pub mod inline {
    #[inline]
    pub fn scale(x: u32) -> u32 {
        x.wrapping_mul(3) // E5
    }
}
pub mod plain {
    pub fn plain(x: u32) -> u32 { // E6
        let r = if x > 10 { x - 10 } else { x + 10 }; // E3
        r.into()
    }
}
pub mod panics {
    pub fn checked_div(a: u32, b: u32) -> u32 {
        if b == 0 {
            panic!("div by zero");
        }
        a / b
    }
    pub fn line_here() -> u32 {
        std::panic::Location::caller().line()
    }
}
pub mod point {
    pub struct Point {
        pub x: u32,
        pub y: u32, // E7
    }
    impl Point {
        pub fn new(x: u32, y: u32) -> Point { Point { x, y } } // E7
    }
    pub fn norm1(p: &Point) -> u32 { p.x + p.y }
}
pub mod shape {
    pub trait Shape { fn area(&self) -> u32; }
    impl Shape for u8 { fn area(&self) -> u32 { 1 } }
    // E8
}
pub use {generic::*, inline::*, panics::*, plain::*, point::*, shape::*};
