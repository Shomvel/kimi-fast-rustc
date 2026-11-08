#!/usr/bin/env bash
# 在 WORKDIR/src 下写出测量用的小程序（多个模块，含泛型、闭包、迭代器、enum、?、dyn、循环）。
# 用法：sample-crate.sh WORKDIR
set -euo pipefail
work=${1:?usage: sample-crate.sh WORKDIR}
mkdir -p "$work/src"
cat > "$work/src/main.rs" <<'RS'
mod iters {
    pub fn sum_sq(v: &[u32]) -> u64 { v.iter().map(|&x| (x as u64) * (x as u64)).sum() }
    pub fn add_assign(a: &mut [f32], b: &[f32]) { for (x, y) in a.iter_mut().zip(b) { *x += *y; } }
}
mod enums {
    #[derive(Debug, Clone, PartialEq)]
    pub enum Shape { Circle(f64), Rect { w: f64, h: f64 }, Empty }
    pub fn area(s: &Shape) -> f64 {
        match s { Shape::Circle(r) => 3.14 * r * r, Shape::Rect { w, h } => w * h, Shape::Empty => 0.0 }
    }
    pub fn first_rect(v: &[Shape]) -> Option<&Shape> { v.iter().find(|s| matches!(s, Shape::Rect { .. })) }
}
mod errors {
    #[derive(Debug)]
    pub enum E { Parse(std::num::ParseIntError), Neg(i64) }
    impl From<std::num::ParseIntError> for E { fn from(e: std::num::ParseIntError) -> Self { E::Parse(e) } }
    pub fn parse_all(xs: &[&str]) -> Result<Vec<i64>, E> {
        let mut out = Vec::new();
        for x in xs { let n: i64 = x.trim().parse()?; if n < 0 { return Err(E::Neg(n)); } out.push(n); }
        Ok(out)
    }
}
mod dynamic {
    pub trait Op { fn apply(&self, x: i64) -> i64; }
    pub struct Add(pub i64);
    pub struct Mul(pub i64);
    impl Op for Add { fn apply(&self, x: i64) -> i64 { x + self.0 } }
    impl Op for Mul { fn apply(&self, x: i64) -> i64 { x * self.0 } }
    pub fn run(ops: &[Box<dyn Op>], x: i64) -> i64 { ops.iter().fold(x, |acc, op| op.apply(acc)) }
}
mod generic {
    use std::collections::HashMap;
    pub fn count<T: std::hash::Hash + Eq + Clone>(v: &[T]) -> HashMap<T, usize> {
        let mut m = HashMap::new();
        for x in v { *m.entry(x.clone()).or_insert(0) += 1; }
        m
    }
    pub fn largest<T: PartialOrd + Copy>(v: &[T]) -> Option<T> {
        let mut it = v.iter().copied();
        let first = it.next()?;
        Some(it.fold(first, |a, b| if b > a { b } else { a }))
    }
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n = args.len() as u32 + 100;
    let v: Vec<u32> = (0..n).collect();
    let mut a: Vec<f32> = (0..n).map(|x| x as f32).collect();
    let b = a.clone();
    iters::add_assign(&mut a, &b);
    let shapes = vec![enums::Shape::Circle(1.0), enums::Shape::Rect { w: 2.0, h: 3.0 }, enums::Shape::Empty];
    let total: f64 = shapes.iter().map(enums::area).sum();
    let parsed = errors::parse_all(&["1", "2", " 3"]);
    let ops: Vec<Box<dyn dynamic::Op>> = vec![Box::new(dynamic::Add(3)), Box::new(dynamic::Mul(2))];
    let words = vec!["a", "b", "a"];
    let counts = generic::count(&words);
    println!("{} {} {} {:?} {:?} {} {:?} {:?}", iters::sum_sq(&v), a[3], total,
             enums::first_rect(&shapes), parsed, dynamic::run(&ops, 5), counts.get("a"),
             generic::largest(&[1.5f64, 0.5]));
}
RS
