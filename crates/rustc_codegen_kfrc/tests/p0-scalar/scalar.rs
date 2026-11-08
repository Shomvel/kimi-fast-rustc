//! P0 scalar workload: compiled by the kfrc backend, called from a driver
//! compiled by LLVM. Every function is `#[inline(never)]` so the driver links
//! to kfrc's code instead of instantiating its own copy.

#![allow(clippy::all)]

// --- T01 regression shapes (docs/plan/m0/t01-reproduce-miscompile.md) ---

#[inline(never)]
pub fn t01_loop_in_if_then(f: bool) -> i64 {
    let mut x = 1;
    let y = if f {
        loop {
            x = 2;
            break 10;
        }
    } else {
        20
    };
    x + y
}

#[inline(never)]
pub fn t01_loop_in_if_else(f: bool) -> i64 {
    let mut x = 1;
    let y = if f {
        10
    } else {
        loop {
            x = 2;
            break 20;
        }
    };
    x + y
}

#[inline(never)]
pub fn t01_loop_in_match(v: i64) -> i64 {
    let mut x = 1;
    let y = match v {
        0 => loop {
            x = 2;
            break 10;
        },
        _ => 20,
    };
    x + y
}

#[inline(never)]
pub fn t01_nested_if_expression(f: bool, g: bool) -> i64 {
    let mut x = 1;
    let y = if f {
        if g {
            x = 2;
            1
        } else {
            0
        }
    } else {
        x = 3;
        if g { 1 } else { 0 }
    };
    x + y
}

// --- control flow ---

#[inline(never)]
pub fn add_if(f: bool, a: i64) -> i64 {
    let mut x = a;
    if f {
        x = x + 1;
    }
    x
}

#[inline(never)]
pub fn sum_to(n: i64) -> i64 {
    let mut i = 0;
    let mut s = 0;
    while i < n {
        s += i;
        i += 1;
    }
    s
}

#[inline(never)]
pub fn collatz_steps(mut n: u64) -> u32 {
    let mut steps = 0;
    while n != 1 {
        if n & 1 == 0 {
            n >>= 1;
        } else {
            n = 3 * n + 1;
        }
        steps += 1;
    }
    steps
}

#[inline(never)]
pub fn labeled(n: i32) -> i32 {
    let mut total = 0;
    let mut i = 0;
    'outer: loop {
        if i >= n {
            break;
        }
        let mut j = 0;
        loop {
            if j > i {
                break;
            }
            if (i + j) & 3 == 3 {
                j += 1;
                continue;
            }
            if i * j > 40 {
                break 'outer;
            }
            total += i * 10 + j;
            j += 1;
        }
        i += 1;
    }
    total
}

#[inline(never)]
pub fn classify(x: i32) -> i32 {
    match x {
        i32::MIN..=-1 => -1,
        0 => 0,
        1..=9 => 1,
        10 | 20 | 30 => 3,
        _ => 2,
    }
}

#[inline(never)]
pub fn logic(a: bool, b: bool, c: bool) -> bool {
    a && b || !c
}

#[inline(never)]
pub fn early_return(n: i64) -> i64 {
    let mut i = 0;
    loop {
        if i * i > n {
            return i;
        }
        i += 1;
    }
}

#[inline(never)]
pub fn shadowing(a: i64) -> i64 {
    let x = a;
    let x = x * 2;
    let r = {
        let x = x + 1;
        x * 3
    };
    r + x
}

// --- calls ---

#[inline(never)]
pub fn fib(n: u32) -> u64 {
    if n < 2 { n as u64 } else { fib(n - 1) + fib(n - 2) }
}

#[inline(never)]
pub fn is_even(n: u32) -> bool {
    if n == 0 { true } else { is_odd(n - 1) }
}

#[inline(never)]
pub fn is_odd(n: u32) -> bool {
    if n == 0 { false } else { is_even(n - 1) }
}

#[inline(never)]
pub fn many_args(a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64, h: i64) -> i64 {
    a - b + c * 2 - d + e * 3 - f + g * 5 - h
}

#[inline(never)]
pub fn call_many(x: i64) -> i64 {
    many_args(x, x + 1, x + 2, x + 3, x + 4, x + 5, x + 6, x + 7) + many_args(1, 2, 3, 4, 5, 6, 7, x)
}

#[inline(never)]
pub fn live_across_calls(a: i64, b: i64) -> i64 {
    let p = a * 3;
    let q = b - 7;
    let r = fib(10) as i64;
    let s = add_if(a > b, p);
    p + q + r + s
}

#[inline(never)]
pub fn unit_call(x: i64) -> i64 {
    nothing();
    x
}

#[inline(never)]
fn nothing() {}

// --- integer widths, casts, shifts ---

#[inline(never)]
pub fn wrap_u8(a: u8, b: u8) -> u8 {
    a + b
}

#[inline(never)]
pub fn wrap_i8(a: i8, b: i8) -> i8 {
    a - b
}

#[inline(never)]
pub fn mul_i16(a: i16, b: i16) -> i16 {
    a * b
}

#[inline(never)]
pub fn neg_i32(a: i32) -> i32 {
    -a
}

#[inline(never)]
pub fn not_u16(a: u16) -> u16 {
    !a
}

#[inline(never)]
pub fn shifts(a: i32, b: u32, s: u32) -> i64 {
    let left = a << (s & 31);
    let arithmetic = a >> (s & 31);
    let logical = b >> (s & 31);
    left as i64 * 3 + arithmetic as i64 * 5 + logical as i64
}

#[inline(never)]
pub fn casts(x: i64) -> i64 {
    let a = x as u8 as i64;
    let b = x as i8 as i64;
    let c = x as u16 as i16 as i64;
    let d = x as i32 as u32 as i64;
    let e = (x > 0) as i64;
    a + b * 3 + c * 5 + d * 7 + e * 11
}

#[inline(never)]
pub fn compare_unsigned(a: u32, b: u32) -> i32 {
    (a < b) as i32 + (a <= b) as i32 * 2 + (a > b) as i32 * 4 + (a >= b) as i32 * 8
}

#[inline(never)]
pub fn compare_signed(a: i16, b: i16) -> i32 {
    (a < b) as i32 + (a <= b) as i32 * 2 + (a > b) as i32 * 4 + (a >= b) as i32 * 8
}

#[inline(never)]
pub fn char_code(c: char) -> u32 {
    c as u32 + 1
}

#[inline(never)]
pub fn bits(a: u64, b: u64) -> u64 {
    (a & b) ^ (a | b.rotate_left_free()) ^ !a
}

trait RotateFree {
    fn rotate_left_free(self) -> Self;
}

impl RotateFree for u64 {
    #[inline(never)]
    fn rotate_left_free(self) -> u64 {
        (self << 7) | (self >> 57)
    }
}
