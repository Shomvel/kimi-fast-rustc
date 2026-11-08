include!("t12_base.rs");
fn mk<T: Tr>() -> Option<T::A> { None }
fn main() { let _ = mk::<_>(); }
