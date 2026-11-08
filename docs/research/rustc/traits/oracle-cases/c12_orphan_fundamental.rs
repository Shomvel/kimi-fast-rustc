struct Local;
impl std::fmt::Display for Box<Local> { fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) } }
impl std::fmt::Display for &Local { fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) } }
fn main() {}
