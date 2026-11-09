#![allow(unused, non_snake_case, unconditional_panic)]
// examples/hello.rs::main
pub mod case_000 {
#[inline(never)] pub fn test_fn() {
    println!("kfrc-ok");
}

}
// examples/integer_syntax.kfr::main
pub mod case_001 {
fn adjust(mut value: isize) -> isize {
    let mut count = 0;
    while count < 2 {
        value &= 0x7f;
        value ^= 0x20;
        value <<= 1;
        value >>= 1;
        value |= 1;
        count += 1;
    }
    return value;
}

#[inline(never)] pub fn test_fn() -> i64 {
    let mut value: u64 = 0x8000_0000_0000_0000u64;
    value >>= 63;
    value <<= 6;
    value |= 2;
    value ^= 0x68;
    let signed: isize = adjust(0xff);
    return (value as i64) + (signed as i64) - 0b111_1111;
}

}
// examples/native_bitwise.kfr::main
pub mod case_002 {
#[inline(never)] pub fn test_fn() -> i64 {
    let value = 5;
    let shifted = (value << 4) | 2;
    let masked = shifted & 126;
    let result = masked ^ 80;
    let negated = -result;
    return (negated >> 1) + 43;
}

}
// examples/native_bool.kfr::main
pub mod case_003 {
fn is_positive(value: i64) -> bool {
    return value > 0;
}

#[inline(never)] pub fn test_fn() -> i64 {
    let positive = is_positive(42);
    let valid = positive && !false;
    return if valid { 42 } else { 0 };
}

}
// examples/native_bool_compare.kfr::main
pub mod case_004 {
#[inline(never)] pub fn test_fn() -> i64 {
    let mut value = false;
    let mut changes = 0;
    let mut index = 0;
    while index < 4 {
        let next = !value;
        if next != value {
            changes += 1;
        }
        value = next;
        index += 1;
    }
    let unchanged = value == false;
    let wrong_count = changes != 4;
    return if unchanged && !wrong_count { 42 } else { 0 };
}

}
// examples/native_boolean_inverse.kfr::main
pub mod case_005 {
#[inline(never)] pub fn test_fn() -> i64 {
    let signed = !(6 < 4);
    let unsigned = !(0xffff_ffff_ffff_ffffu64 < 1);
    return if signed && unsigned { 42 } else { 0 };
}

}
// examples/native_break_continue.kfr::main
pub mod case_006 {
#[inline(never)] pub fn test_fn() -> i64 {
    let mut sum = 0;
    let mut i = 0;
    while true {
        i += 1;
        if i > 10 {
            break;
        }
        if i % 2 == 1 {
            continue;
        }
        sum += i;
    }
    sum += 12;
    return sum;
}

}
// examples/native_call.kfr::main
pub mod case_007 {
fn answer() -> i64 {
    return 42;
}

#[inline(never)] pub fn test_fn() -> i64 {
    return answer();
}

}
// examples/native_cast.kfr::main
pub mod case_008 {
fn convert(value: i64) -> u64 {
    return value as u64;
}

#[inline(never)] pub fn test_fn() -> i64 {
    let highest_bit = convert(-1) >> 63;
    return (highest_bit == 1) as i64 + 41;
}

}
// examples/native_compare_immediate.kfr::main
pub mod case_009 {
fn greater(value: i64) -> i64 {
    return if 2 < value { 42 } else { 0 };
}

#[inline(never)] pub fn test_fn() -> i64 {
    return greater(3);
}

}
// examples/native_const_expression.kfr::main
pub mod case_010 {
pub const ANSWER: i64 = (BASE + OFFSET) * 2;
const BASE: i64 = LIMIT - 1;
const OFFSET: i64 = 1;
const LIMIT: i64 = 21;
const CHECKED: bool = ANSWER == 42 && !false;

#[inline(never)] pub fn test_fn() -> i64 {
    if CHECKED {
        return ANSWER;
    }
    return 0;
}

}
// examples/native_const_fn.kfr::main
pub mod case_011 {
#[inline(never)] pub const fn test_fn() -> i64 {
    return 42;
}

}
// examples/native_const_item.kfr::main
pub mod case_012 {
pub const ANSWER: i64 = 0x2a;

#[inline(never)] pub fn test_fn() -> i64 {
    return ANSWER;
}

}
// examples/native_early_return.kfr::main
pub mod case_013 {
fn classify(value: i64) -> i64 {
    if value > 40 {
        return 42;
    } else if value < 0 {
        return 0;
    }
    return 1;
}

#[inline(never)] pub fn test_fn() -> i64 {
    let mut i = 0;
    while true {
        i += 1;
        if i == 3 {
            return classify(42);
        }
    }
    return 0;
}

}
// examples/native_else_if.kfr::main
pub mod case_014 {
fn normalize(mut value: i64) -> i64 {
    value += 5;
    value *= 3;
    if value >= 100 {
        value = 100;
    } else if value >= 50 {
        value -= 21;
    } else if value < 0 {
        value = 0;
    }
    return value;
}

#[inline(never)] pub fn test_fn() -> i64 {
    return normalize(16);
}

}
// examples/native_expressions.kfr::main
pub mod case_015 {
fn spin(mut value: i64) {
    while value > 0 {
        value -= 1;
    }
    return;
}

fn answer() -> i64 {
    return 42;
}

#[inline(never)] pub fn test_fn() -> i64 {
    spin(3);
    answer();
    1 + 2;
    3 < 4;
    return answer();
}

}
// examples/native_for.kfr::main
pub mod case_016 {
fn start() -> usize {
    return 1;
}

fn end() -> usize {
    return 6;
}

#[inline(never)] pub fn test_fn() -> u64 {
    let mut total = 0usize;
    for i in start()..end() {
        if i % 2 == 0 {
            continue;
        }
        total += i;
    }
    return (total + 33) as u64;
}

}
// examples/native_id.kfr::main
pub mod case_017 {
fn id(x: i64) -> i64 {
    return x;
}

#[inline(never)] pub fn test_fn() -> i64 {
    return id(42);
}

}
// examples/native_if.kfr::main
pub mod case_018 {
#[inline(never)] pub fn test_fn() -> i64 {
    return if 2 > 1 { 42 } else { 0 };
}

}
// examples/native_inclusive.kfr::main
pub mod case_019 {
#[inline(never)] pub fn test_fn() -> u64 {
    let mut total = 0usize;
    for i in 1..=6usize {
        if i % 2 == 0 {
            continue;
        }
        total += i;
    }
    return (total + 33) as u64;
}

}
// examples/native_loop_value.kfr::main
pub mod case_020 {
#[inline(never)] pub fn test_fn() -> i64 {
    let answer = loop {
        break 40 + 2;
    };
    answer
}

}
// examples/native_main.kfr::main
pub mod case_021 {
#[inline(never)] pub fn test_fn() -> i64 {
    let base = 40;
    return base + 2;
}

}
// examples/native_match.kfr::main
pub mod case_022 {
#[inline(never)] pub fn test_fn() -> i64 {
    return match 2 {
        0 => 0,
        1 => 21,
        2 => { 40 + 2 },
        _ => -1,
    };
}

}
// examples/native_match_range.kfr::main
pub mod case_023 {
#[inline(never)] pub fn test_fn() -> i64 {
    let value = 42;
    return match value {
        -10..=10 => 0,
        40..=50 if value % 2 == 0 => 42,
        _ => -1,
    };
}

}
// examples/native_nested_while.kfr::main
pub mod case_024 {
#[inline(never)] pub fn test_fn() -> i64 {
    let mut total = 0;
    let mut i = 0;
    while i < 4 {
        let mut j = 0;
        while j < i {
            total = total + j;
            j = j + 1;
        }
        i = i + 1;
    }
    return total;
}

}
// examples/native_not.kfr::main
pub mod case_025 {
#[inline(never)] pub fn test_fn() -> i64 {
    let signed = !(-43);
    let unsigned = !0xffff_ffff_ffff_ffd5u64;
    return if !false { signed + (unsigned as i64) - 42 } else { 0 };
}

}
// examples/native_recursion.kfr::main
pub mod case_026 {
#[inline(never)] pub fn test_fn() -> i64 {
    return if even(10) { sum_down(9) - 3 } else { 0 };
}

fn sum_down(value: i64) -> i64 {
    if value == 0 {
        return 0;
    }
    return value + sum_down(value - 1);
}

fn even(value: u64) -> bool {
    if value == 0 {
        return true;
    }
    return odd(value - 1);
}

fn odd(value: u64) -> bool {
    if value == 0 {
        return false;
    }
    return even(value - 1);
}

}
// examples/native_remainder.kfr::main
pub mod case_027 {
#[inline(never)] pub fn test_fn() -> u64 {
    let remainder = 0xffff_ffff_ffff_ffffu64 % 64;
    let quotient = 255u64 / 64;
    let high = 0x8000_0000_0000_0000u64;
    let high_remainder = (high - 1) % high;
    return if high_remainder == high - 1 {
        remainder - quotient - 18
    } else {
        0
    };
}

}
// examples/native_second.kfr::main
pub mod case_028 {
#[inline(never)] pub fn test_fn() -> i64 {
    let left = 6;
    return left * 7;
}

}
// examples/native_shadowing.kfr::main
pub mod case_029 {
#[inline(never)] pub fn test_fn() -> i64 {
    let mut x = 40;
    for x in 0..3 {
        let x = x + 1;
        if x == 2 {
            let x = 100;
        }
    }
    let x = x + 1;
    let mut i = 0;
    while i < 1 {
        let x = 0;
        i += 1;
    }
    return x + 1;
}

}
// examples/native_signed_division.kfr::main
pub mod case_030 {
#[inline(never)] pub fn test_fn() -> i64 {
    let negative = -85;
    let positive = 85;
    return negative / 2 + positive / 2 + 42;
}

}
// examples/native_statement_if.kfr::main
pub mod case_031 {
#[inline(never)] pub fn test_fn() -> i64 {
    let mut sum = 0;
    let mut i = 0;
    while i < 10 {
        if i % 2 == 0 {
            sum = sum + i;
        } else {
            sum = sum + 0;
        }
        i = i + 1;
    }
    if sum == 20 {
        sum = sum + 22;
    }
    return sum;
}

}
// examples/native_strength.kfr::main
pub mod case_032 {
fn scale(value: i64) -> i64 {
    return value * 8;
}

fn reduce(value: u64) -> u64 {
    return value / 8;
}

#[inline(never)] pub fn test_fn() -> i64 {
    return scale(5) + reduce(16) as i64;
}

}
// examples/native_sum8.kfr::main
pub mod case_033 {
fn sum8(a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64, h: i64) -> i64 {
    return a + b + c + d + e + f + g + h;
}

#[inline(never)] pub fn test_fn() -> i64 {
    return sum8(1, 2, 3, 4, 5, 6, 7, 8);
}

}
// examples/native_tail.kfr::main
pub mod case_034 {
fn sum() -> i64 {
    let mut total = 0;
    for i in 0..5 {
        total += i;
    }
    total + 32
}

#[inline(never)] pub fn test_fn() -> i64 {
    sum()
}

}
// examples/native_u32_immediate.kfr::main
pub mod case_035 {
#[inline(never)] pub fn test_fn() -> u64 {
    let value = 0x8000_0000u64;
    return if value > 0x7fff_ffffu64 { 42u64 } else { 0 };
}

}
// examples/native_u64.kfr::main
pub mod case_036 {
fn compute(value: u64) -> u64 {
    return (value >> 63) + (value % 3) + 41;
}

#[inline(never)] pub fn test_fn() -> u64 {
    return compute(18446744073709551615u64);
}

}
// examples/native_unary_strength.kfr::main
pub mod case_037 {
#[inline(never)] pub fn test_fn() -> i64 {
    let negated = 0 - (-40);
    let multiplied = 1 * -2;
    let inverted = 0 ^ -1;
    return negated + multiplied + inverted + 5;
}

}
// examples/native_unit.rs::main
pub mod case_038 {
#[inline(never)] pub fn test_fn() {}

}
// examples/native_visibility.kfr::main
pub mod case_039 {
#[inline(never)] pub fn test_fn() -> i64 {
    return 40 + 2;
}

pub(crate) fn crate_visible() -> i64 {
    return 42;
}

pub(self) fn self_visible() -> i64 {
    return 42;
}

pub(super) fn super_visible() -> i64 {
    return 42;
}

}
// examples/native_while.kfr::main
pub mod case_040 {
fn sum_to(limit: i64) -> i64 {
    let mut sum = 0;
    let mut i = 1;
    while i <= limit {
        sum = sum + i;
        i = i + 1;
    }
    return sum;
}

#[inline(never)] pub fn test_fn() -> i64 {
    return sum_to(10);
}

}
