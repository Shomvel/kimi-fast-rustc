struct Local;
trait Mine {}
impl<T: std::fmt::Display> Mine for T {}
impl Mine for Local {}
impl Mine for Vec<u8> {}
fn main() {}
