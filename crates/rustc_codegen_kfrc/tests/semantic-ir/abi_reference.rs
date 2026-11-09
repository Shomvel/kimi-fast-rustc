#![allow(dead_code)]

pub struct Pair {
    pub x: u64,
    pub y: u64,
}

pub struct Big {
    pub head: (u64, u64),
    pub middle: Pair,
    pub tail: u64,
}

#[inline(never)]
pub fn foreign_pair(pair: Pair, bias: u64) -> Pair {
    Pair {
        x: pair.y + bias,
        y: pair.x ^ bias,
    }
}

#[inline(never)]
pub fn foreign_big(value: Big, bias: u64) -> Big {
    Big {
        head: (value.tail + bias, value.head.0 ^ value.middle.y),
        middle: Pair {
            x: value.middle.x + bias,
            y: value.head.1,
        },
        tail: value.middle.y,
    }
}
