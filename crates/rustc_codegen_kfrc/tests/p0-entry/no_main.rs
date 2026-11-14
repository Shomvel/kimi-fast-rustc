// Exercises the executable linker and allocator module without the Rust entry
// adapter, so failures in those two boundaries can be diagnosed independently.
#![no_main]

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    37
}
