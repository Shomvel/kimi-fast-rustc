trait Set<T> { fn contains(&self, _: T) -> bool; }
impl<'a, T, S> Set<&'a [T]> for S where T: Copy, S: Set<T> {
    fn contains(&self, bits: &[T]) -> bool { bits.iter().all(|&bit| self.contains(bit)) }
}
fn main() { let bits: &[_] = &[0, 1]; 0.contains(bits); }
