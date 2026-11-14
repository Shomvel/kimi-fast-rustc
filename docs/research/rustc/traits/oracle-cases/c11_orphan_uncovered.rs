struct Local;
impl<T> std::ops::Add<Local> for T { type Output = (); fn add(self, _: Local) {} }
fn main() {}
