//! RustCall arguments may be scalar, zero-sized, or indirect CTFE tuples.

use std::marker::PhantomData;
use std::ops::{Fn, FnMut, FnOnce};

const SCALAR: (i64,) = (37,);
const ZERO_SIZED: ((), PhantomData<u64>) = ((), PhantomData);
const PAIR: (i64, i64) = (13, -7);
const INDIRECT: ([i64; 3], u8) = ([17, -5, 23], 11);
const VALUE: i64 = 41;
const POINTER: (&i64,) = (&VALUE,);

#[inline(never)]
fn scalar(value: i64) -> i64 {
    value.wrapping_mul(3)
}

#[inline(never)]
fn zero_sized(_: (), _: PhantomData<u64>) -> i64 {
    19
}

#[inline(never)]
fn pair(left: i64, right: i64) -> i64 {
    left.wrapping_mul(5).wrapping_add(right)
}

#[inline(never)]
fn indirect(values: [i64; 3], tag: u8) -> i64 {
    values[0]
        .wrapping_add(values[1].wrapping_mul(3))
        .wrapping_add(values[2].wrapping_mul(5))
        .wrapping_add(tag as i64)
}

#[inline(never)]
fn pointer(value: &i64) -> i64 {
    value.wrapping_add(7)
}

#[inline(never)]
pub fn constant_arguments(capture: i64) -> i64 {
    let function = |value: i64| value.wrapping_add(capture);
    let shared: &dyn Fn(i64) -> i64 = &function;
    let mut mutable = |value: i64| value.wrapping_sub(capture);
    let once: Box<dyn FnOnce() -> i64> = Box::new(move || capture.wrapping_mul(7));
    FnOnce::call_once(scalar, SCALAR)
        .wrapping_add(FnOnce::call_once(zero_sized, ZERO_SIZED))
        .wrapping_add(FnOnce::call_once(pair, PAIR))
        .wrapping_add(FnOnce::call_once(indirect, INDIRECT))
        .wrapping_add(FnOnce::call_once(pointer, POINTER))
        .wrapping_add(Fn::call(shared, SCALAR))
        .wrapping_add(FnMut::call_mut(&mut mutable, SCALAR))
        .wrapping_add(FnOnce::call_once(once, ()))
}

struct DropCapture {
    value: i64,
    marker: *mut i64,
}

impl Drop for DropCapture {
    fn drop(&mut self) {
        unsafe { *self.marker = self.value.wrapping_add(29) };
    }
}

#[inline(never)]
pub fn boxed_once_arguments(capture: i64) -> i64 {
    let mut marker = 0;
    let token = DropCapture {
        value: capture,
        marker: &mut marker,
    };
    let callback: Box<dyn FnOnce(i64, (i64, i64)) -> (i64, i64, i64)> =
        Box::new(move |argument: i64, pair: (i64, i64)| {
            let value = token.value;
            drop(token);
            (
                value.wrapping_add(argument),
                pair.0.wrapping_mul(3),
                pair.1.wrapping_mul(5),
            )
        });
    let result = callback(13, (17, 19));
    result
        .0
        .wrapping_add(result.1)
        .wrapping_add(result.2)
        .wrapping_add(marker)
}

#[inline(never)]
pub fn boxed_once_zero_sized() -> i64 {
    let callback: Box<dyn FnOnce() -> i64> = Box::new(|| 71);
    callback()
}

trait Consume {
    fn consume(self: Box<Self>, increment: i64) -> i64;
}

struct Consumed(i64);

impl Consume for Consumed {
    fn consume(self: Box<Self>, increment: i64) -> i64 {
        self.0.wrapping_mul(3).wrapping_add(increment)
    }
}

#[inline(never)]
pub fn boxed_method_receiver(capture: i64) -> i64 {
    let value: Box<dyn Consume> = Box::new(Consumed(capture));
    value.consume(23)
}
