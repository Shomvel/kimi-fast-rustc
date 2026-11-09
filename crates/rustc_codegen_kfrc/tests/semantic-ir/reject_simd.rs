#![feature(repr_simd)]
#[repr(simd)]
pub struct Vector(pub [u32; 4]);
#[inline(never)]
pub fn vector_value(value: Vector) -> Vector {
    value
}
