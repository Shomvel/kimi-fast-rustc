struct S;
trait Tr2 {}
impl Tr2 for S where S: Tr2 {}
fn need<T: Tr2>() {}
fn main() { need::<S>(); }
