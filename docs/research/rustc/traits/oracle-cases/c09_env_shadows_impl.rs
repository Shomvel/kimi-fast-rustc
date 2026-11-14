trait Foo { type Assoc; }
trait Bar {}
impl<T> Foo for T { type Assoc = i32; }
impl<T> Bar for T where T: Foo<Assoc = i32> {}
fn require_bar<T: Bar>() {}
fn foo<T: Foo>() { require_bar::<T>(); }
fn main() {}
