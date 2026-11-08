struct Foo<T>(T);
unsafe impl<T: SendIndir<Foo<T>>> Send for Foo<T> {}
trait SendIndir<T> {}
impl<T, U: Send> SendIndir<U> for T {}
fn is_send<T: Send>() {}
fn main() { is_send::<Foo<u32>>(); }
