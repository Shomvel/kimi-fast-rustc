// W02: iterator chain with closures over a slice.
#[inline(never)]
pub fn sum_scaled(xs: &[u32], k: u64) -> u64 {
    xs.iter()
        .filter(|x| **x % 3 != 0)
        .map(|&x| x as u64 * k)
        .sum::<u64>()
}

fn main() {
    let v: Vec<u32> = (0..1000).collect();
    let k = std::env::args().count() as u64;
    println!("{}", sum_scaled(&v, k));
}
