include!("t12_base.rs");
impl<T: Copy> Tr for T { type A = (); fn get(&self) {} }
fn main() {}
