struct List(Option<Box<List>>);
fn is_send<T: Send>() {}
fn main() { is_send::<List>(); }
