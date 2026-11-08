//! Calls every function of the `scalar` crate over fixed input grids and
//! prints one line per call. Compiled by LLVM in both arms; only `scalar`
//! changes backend.

use scalar::*;

const WIDE: [i64; 16] = [
    i64::MIN, -1_000_000, -1000, -129, -128, -1, 0, 1, 2, 7, 127, 128, 255, 256, 1_000_000, i64::MAX,
];

fn main() {
    for f in [false, true] {
        println!("t01_loop_in_if_then({f}) = {}", t01_loop_in_if_then(f));
        println!("t01_loop_in_if_else({f}) = {}", t01_loop_in_if_else(f));
        for g in [false, true] {
            println!("t01_nested_if_expression({f}, {g}) = {}", t01_nested_if_expression(f, g));
        }
    }
    for v in -2..12 {
        println!("t01_loop_in_match({v}) = {}", t01_loop_in_match(v));
    }
    for a in WIDE {
        for f in [false, true] {
            println!("add_if({f}, {a}) = {}", add_if(f, a));
        }
        println!("shadowing({a}) = {}", shadowing(a));
        println!("casts({a}) = {}", casts(a));
        println!("call_many({a}) = {}", call_many(a));
        println!("unit_call({a}) = {}", unit_call(a));
        for b in [-3i64, 0, 5, 1 << 40] {
            println!("live_across_calls({a}, {b}) = {}", live_across_calls(a, b));
        }
    }
    for n in [-5, 0, 1, 2, 10, 100, 1000] {
        println!("sum_to({n}) = {}", sum_to(n));
    }
    for n in [1u64, 2, 3, 6, 7, 9, 27, 97, 871, 1 << 40] {
        println!("collatz_steps({n}) = {}", collatz_steps(n));
    }
    for n in -1..16 {
        println!("labeled({n}) = {}", labeled(n));
    }
    for x in [i32::MIN, -5, -1, 0, 1, 5, 9, 10, 11, 20, 25, 30, 31, i32::MAX] {
        println!("classify({x}) = {}", classify(x));
        println!("neg_i32({x}) = {}", neg_i32(x));
    }
    for a in [false, true] {
        for b in [false, true] {
            for c in [false, true] {
                println!("logic({a}, {b}, {c}) = {}", logic(a, b, c));
            }
        }
    }
    for n in [0i64, 1, 5, 100, 10_000] {
        println!("early_return({n}) = {}", early_return(n));
    }
    for n in 0..25 {
        println!("fib({n}) = {}", fib(n));
    }
    for n in 0..20 {
        println!("is_even({n}) = {} is_odd({n}) = {}", is_even(n), is_odd(n));
    }
    let narrow8: [u8; 6] = [0, 1, 100, 127, 128, 255];
    for a in narrow8 {
        for b in narrow8 {
            println!("wrap_u8({a}, {b}) = {}", wrap_u8(a, b));
            println!("wrap_i8({}, {}) = {}", a as i8, b as i8, wrap_i8(a as i8, b as i8));
        }
    }
    for a in [i16::MIN, -300, -1, 0, 1, 255, 300, i16::MAX] {
        for b in [i16::MIN, -2, 0, 3, 200, i16::MAX] {
            println!("mul_i16({a}, {b}) = {}", mul_i16(a, b));
            println!("compare_signed({a}, {b}) = {}", compare_signed(a, b));
        }
        println!("not_u16({}) = {}", a as u16, not_u16(a as u16));
    }
    for a in [0u32, 1, 7, 0x8000_0000, u32::MAX] {
        for b in [0u32, 1, 0x7fff_ffff, u32::MAX] {
            println!("compare_unsigned({a}, {b}) = {}", compare_unsigned(a, b));
        }
    }
    for a in [i32::MIN, -77, -1, 0, 1, 77, i32::MAX] {
        for s in [0u32, 1, 5, 31, 32, 33, 63] {
            println!("shifts({a}, {}, {s}) = {}", a as u32, shifts(a, a as u32, s));
        }
    }
    for c in ['\0', 'a', 'Z', 'é', '中', '\u{10FFFF}'] {
        println!("char_code({c:?}) = {}", char_code(c));
    }
    for a in [0u64, 1, 0xdead_beef, u64::MAX] {
        for b in [0u64, 3, 0x0123_4567_89ab_cdef] {
            println!("bits({a}, {b}) = {}", bits(a, b));
        }
    }
}
