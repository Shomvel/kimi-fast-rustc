#![feature(core_intrinsics, funnel_shifts)]
#![allow(internal_features)]

extern crate abi_reference;
use abi_reference::{Big, Pair};

#[inline(never)]
pub fn tuple_field(x: u64, y: u64) -> u64 {
    let pair = (x, y);
    pair.0
}

#[inline(never)]
pub fn nested_fields(x: u64, y: u64, z: u64) -> u64 {
    let mut value = ((x, y), (z, true));
    let snapshot = value;
    value.0.1 = z;
    value.1.0 = x;
    snapshot.0.1 ^ value.0.1 ^ snapshot.1.0 ^ value.1.0
}

#[inline(never)]
pub fn tuple_branch(x: u64, y: u64, choose: bool) -> (u64, u64) {
    let mut pair = if choose { (x, y) } else { (y, x) };
    pair.1 = pair.0 + pair.1;
    pair
}

#[inline(never)]
pub fn tuple_loop(x: u64, y: u64, count: u64) -> (u64, u64) {
    let mut pair = (x, y);
    let mut i = 0;
    while i < count {
        pair = (pair.1, pair.0 + pair.1);
        i += 1;
    }
    pair
}

#[inline(never)]
pub fn pair_roundtrip(value: Pair, bias: u64) -> Pair {
    let snapshot = Pair {
        x: value.x,
        y: value.y,
    };
    let returned = abi_reference::foreign_pair(snapshot, bias);
    Pair {
        x: returned.x ^ value.x,
        y: returned.y + value.y,
    }
}

#[inline(never)]
pub fn big_roundtrip(value: Big, bias: u64) -> Big {
    let snapshot = Big {
        head: value.head,
        middle: Pair {
            x: value.middle.x,
            y: value.middle.y,
        },
        tail: value.tail,
    };
    let returned = abi_reference::foreign_big(snapshot, bias);
    Big {
        head: (returned.head.0 ^ value.tail, returned.head.1),
        middle: Pair {
            x: returned.middle.x,
            y: returned.middle.y + value.head.1,
        },
        tail: returned.tail ^ value.middle.x,
    }
}

#[inline(never)]
pub fn local_call_snapshot(x: u64, y: u64) -> u64 {
    let mut original = (x, y);
    let result = tuple_branch(original.0, original.1, true);
    original.0 = y;
    result.0 ^ result.1 ^ original.0
}

#[inline(never)]
pub fn array_roundtrip(values: [u64; 3], replacement: u64) -> [u64; 3] {
    let [a, b, c] = values;
    [c ^ replacement, b + replacement, a]
}

#[inline(never)]
pub fn choose_slice<'a>(a: &'a [u64], b: &'a [u64], choose: bool) -> &'a [u64] {
    if choose { a } else { b }
}

#[repr(u8)]
pub enum KnownVariant {
    Empty = 3,
    Data(u64) = 9,
}

#[inline(never)]
pub fn known_payload(x: u64) -> u64 {
    let value = KnownVariant::Data(x);
    match value {
        KnownVariant::Empty => 71,
        KnownVariant::Data(payload) => payload + 1,
    }
}

#[inline(never)]
pub fn known_empty() -> u64 {
    let value = KnownVariant::Empty;
    match value {
        KnownVariant::Empty => 71,
        KnownVariant::Data(payload) => payload + 1,
    }
}

pub fn inline_helper(x: u64, y: u64, choose: bool) -> (u64, u64) {
    if choose {
        (x + y, x ^ y)
    } else {
        (x ^ y, x + y)
    }
}

#[inline(never)]
pub fn inline_caller(x: u64, y: u64, choose: bool) -> u64 {
    let pair = inline_helper(x, y, choose);
    pair.0 + pair.1 + x
}

#[path = "support_borrow.rs"]
pub mod borrow_probe;
#[path = "support_index.rs"]
pub mod index_probe;
#[path = "support_niche.rs"]
pub mod niche_probe;
#[path = "support_variant.rs"]
pub mod variant_probe;

#[path = "dynamic.rs"]
pub mod dynamic_probe;

#[path = "support_direct_tags.rs"]
pub mod direct_tags_probe;

#[path = "support_lp3_demand.rs"]
pub mod lp3_demand_probe;

#[path = "support_rs3_conditions.rs"]
pub mod rs3_conditions_probe;

#[path = "support_rs3_arithmetic.rs"]
pub mod rs3_arithmetic_probe;

#[path = "support_rs3_loops.rs"]
pub mod rs3_loops_probe;

#[path = "support_rs3_ranges.rs"]
pub mod rs3_ranges_probe;

#[path = "support_rs1_intrinsics.rs"]
pub mod rs1_intrinsics_probe;

#[path = "support_rs3_validity.rs"]
pub mod rs3_validity_probe;

#[path = "support_rs5_permissions.rs"]
pub mod rs5_permissions_probe;

#[path = "support_compare_bytes.rs"]
pub mod compare_bytes_probe;
