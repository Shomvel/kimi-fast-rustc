include!("t12_base.rs");
fn main() { let a: u64 = via_eq(3u32); let b: u8 = via_param(true); std::process::exit((a as i32) + (b as i32) - 4); }
