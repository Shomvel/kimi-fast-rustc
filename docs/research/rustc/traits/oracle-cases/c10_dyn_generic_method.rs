trait Bar { fn bar<T>(&self, t: T); }
fn make(t: &impl Bar) -> &dyn Bar { t }
fn main() {}
