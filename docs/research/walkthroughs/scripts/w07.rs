// W07: &dyn / Box<dyn> dispatch, vtable contents, devirtualization, drop via vtable.
use std::io::{self, Write};

pub trait Shape { fn area(&self) -> u64; fn name(&self) -> &'static str { "shape" } }
pub struct Sq(u64);
pub struct Rect { w: u64, h: u64, tag: String } // needs_drop: vtable slot 0 non-null
impl Shape for Sq { fn area(&self) -> u64 { self.0 * self.0 } }
impl Shape for Rect {
    fn area(&self) -> u64 { self.w * self.h + self.tag.len() as u64 }
    fn name(&self) -> &'static str { "rect" }
}
mod dynk { // separate module => separate CGU when codegen-units > 1
    use super::*;
    #[inline(never)]
    pub fn total(xs: &[&dyn Shape]) -> u64 { xs.iter().map(|s| s.area()).sum() } // callee unknown
    #[inline(never)]
    pub fn local_devirt(k: u64) -> u64 {
        let s: Box<dyn Shape> = Box::new(Rect { w: k, h: 3, tag: String::new() }); // unsize
        s.area() // concrete type known after inlining Box::new
    } // Box<dyn Shape> dropped via vtable
    #[inline(never)]
    pub fn emit(w: &mut dyn Write, x: u64) -> io::Result<()> { writeln!(w, "{x}") } // default method
}
fn main() {
    let (r, q) = (Rect { w: 2, h: 5, tag: "r".into() }, Sq(4));
    let xs: [&dyn Shape; 2] = [&r, &q]; // same (Rect, Shape) vtable as in local_devirt
    let mut out: Box<dyn Write> = Box::new(io::stdout());
    let t = dynk::total(&xs) + dynk::local_devirt(std::env::args().count() as u64) + r.name().len() as u64;
    dynk::emit(&mut *out, t).unwrap();
}
