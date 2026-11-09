extern crate abi_reference;
extern crate semantic_values;
use abi_reference::{Big, Pair};
use semantic_values::*;

#[path = "driver_rs3_conditions.rs"]
mod rs3_conditions;

#[path = "driver_rs3_arithmetic.rs"]
mod rs3_arithmetic;

#[path = "driver_rs3_loops.rs"]
mod rs3_loops;

#[path = "driver_rs3_ranges.rs"]
mod rs3_ranges;

#[path = "driver_rs1_intrinsics.rs"]
mod rs1_intrinsics;

#[path = "driver_rs3_validity.rs"]
mod rs3_validity;

#[path = "driver_rs5_permissions.rs"]
mod rs5_permissions;

#[path = "driver_compare_bytes.rs"]
mod compare_bytes;

static DROPPED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[repr(C, align(32))]
struct DynamicValue(u64);
impl dynamic_probe::Value for DynamicValue {
    fn read(&self, bias: u64) -> u64 {
        self.0.wrapping_add(bias)
    }
}
impl Drop for DynamicValue {
    fn drop(&mut self) {
        DROPPED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn main() {
    rs3_conditions::check();
    rs3_arithmetic::check();
    rs3_loops::check();
    rs3_ranges::check();
    rs1_intrinsics::check();
    rs3_validity::check();
    rs5_permissions::check();
    compare_bytes::check();
    check_direct_tags();
    check_lp3_demand();
    check_lp3_transport();
    check_lp3_branch_snapshots();
    check_lp3_owned_storage();
    check_lp3_nested_transport();
    for value in [0u64, 7, u64::MAX] {
        let boxed: Box<dyn dynamic_probe::Value> = Box::new(DynamicValue(value));
        assert_eq!(dynamic_probe::call(&*boxed, 9), value.wrapping_add(9));
        assert_eq!(
            dynamic_probe::size(&*boxed),
            std::mem::size_of::<DynamicValue>()
        );
        assert_eq!(
            dynamic_probe::alignment(&*boxed),
            std::mem::align_of::<DynamicValue>()
        );
        let before = DROPPED.load(std::sync::atomic::Ordering::SeqCst);
        dynamic_probe::destroy(boxed);
        assert_eq!(
            DROPPED.load(std::sync::atomic::Ordering::SeqCst),
            before + 1
        );
        println!("dynamic {value} size=32 align=32 drops={}", before + 1);
    }

    for x in [0u64, 1, 7, u32::MAX as u64, u64::MAX] {
        for y in [0u64, 2, 19, u64::MAX - 1] {
            let z = x ^ y ^ 0xa55a;
            assert_eq!(tuple_field(x, y), x);
            assert_eq!(nested_fields(x, y, z), y ^ x);
            println!(
                "fields {x} {y} {} {}",
                tuple_field(x, y),
                nested_fields(x, y, z)
            );
            for choose in [false, true] {
                let inlined = inline_caller(x, y, choose);
                assert_eq!(
                    inlined,
                    x.wrapping_add(y).wrapping_add(x ^ y).wrapping_add(x)
                );
                println!("inline {choose} {inlined}");
                let pair = tuple_branch(x, y, choose);
                assert_eq!(pair, (if choose { x } else { y }, x.wrapping_add(y)));
                println!("branch {choose} {} {}", pair.0, pair.1);
            }
            for count in [0u64, 1, 2, 9, 65] {
                let mut expected = (x, y);
                for _ in 0..count {
                    expected = (expected.1, expected.0.wrapping_add(expected.1));
                }
                let pair = tuple_loop(x, y, count);
                assert_eq!(pair, expected);
                println!("loop {count} {} {}", pair.0, pair.1);
            }
            let pair = pair_roundtrip(Pair { x, y }, z);
            assert_eq!(
                (pair.x, pair.y),
                (y.wrapping_add(z) ^ x, (x ^ z).wrapping_add(y))
            );
            println!("pair {} {}", pair.x, pair.y);
            let big = big_roundtrip(
                Big {
                    head: (x, y),
                    middle: Pair { x: y, y: z },
                    tail: x ^ z,
                },
                z,
            );
            let expected = (
                (x ^ z).wrapping_add(z) ^ (x ^ z),
                x ^ z,
                y.wrapping_add(z),
                y.wrapping_add(y),
                z ^ y,
            );
            assert_eq!(
                (big.head.0, big.head.1, big.middle.x, big.middle.y, big.tail),
                expected
            );
            println!(
                "big {} {} {} {} {}",
                big.head.0, big.head.1, big.middle.x, big.middle.y, big.tail
            );
            let local = local_call_snapshot(x, y);
            assert_eq!(local, x ^ x.wrapping_add(y) ^ y);
            println!("snapshot {local}");
            let array = array_roundtrip([x, y, z], y);
            assert_eq!(array, [z ^ y, y.wrapping_add(y), x]);
            println!("array {} {} {}", array[0], array[1], array[2]);
        }
    }
    for x in [0u64, 1, 77, u64::MAX] {
        assert_eq!(known_payload(x), x.wrapping_add(1));
        assert_eq!(known_empty(), 71);
        println!("known-variant {x} {} {}", known_payload(x), known_empty());
    }
    for x in [0u64, 1, 77, u64::MAX] {
        let borrowed = borrow_probe::requires_address(x);
        assert_eq!(borrowed, x.wrapping_add(1));
        println!("borrow {x} {borrowed}");
        let data = [x, x.wrapping_add(1), x ^ 0xff];
        for index in 0..3 {
            let got = index_probe::dynamic_index(data, index);
            assert_eq!(got, data[index]);
            println!("index {x} {index} {got}");
        }
        let got = variant_probe::decode_argument(variant_probe::DynamicVariant::Data(x));
        assert_eq!(got, x);
        let empty = variant_probe::decode_argument(variant_probe::DynamicVariant::Empty);
        assert_eq!(empty, 71);
        println!("dynamic-variant {x} {got} {empty}");
        assert!(niche_probe::relay(None).is_none());
        let got = niche_probe::relay(Some(&x)).unwrap();
        assert_eq!(*got, x);
        assert!(std::ptr::eq(got, &x));
        println!("niche {x} {got}");
    }
    let a = [3u64, 5, 7, 9];
    let b = [11u64, 13, 17, 19, 23, 29];
    for left in 0..=a.len() {
        for right in 0..=b.len() {
            for choose in [false, true] {
                let x = &a[..left];
                let y = &b[..right];
                let got = choose_slice(x, y, choose);
                let expected = if choose { x } else { y };
                assert_eq!(got.as_ptr(), expected.as_ptr());
                assert_eq!(got.len(), expected.len());
                assert_eq!(got, expected);
                println!("wide {left} {right} {choose} {}", got.len());
            }
        }
    }
}

fn check_direct_tags() {
    use direct_tags_probe::*;
    let alignments = [
        Alignment::A0,
        Alignment::A1,
        Alignment::A2,
        Alignment::A3,
        Alignment::A4,
        Alignment::A5,
        Alignment::A6,
        Alignment::A7,
        Alignment::A8,
        Alignment::A9,
        Alignment::A10,
        Alignment::A11,
        Alignment::A12,
        Alignment::A13,
        Alignment::A14,
        Alignment::A15,
        Alignment::A16,
        Alignment::A17,
        Alignment::A18,
        Alignment::A19,
        Alignment::A20,
        Alignment::A21,
        Alignment::A22,
        Alignment::A23,
        Alignment::A24,
        Alignment::A25,
        Alignment::A26,
        Alignment::A27,
        Alignment::A28,
        Alignment::A29,
        Alignment::A30,
        Alignment::A31,
        Alignment::A32,
        Alignment::A33,
        Alignment::A34,
        Alignment::A35,
        Alignment::A36,
        Alignment::A37,
        Alignment::A38,
        Alignment::A39,
        Alignment::A40,
        Alignment::A41,
        Alignment::A42,
        Alignment::A43,
        Alignment::A44,
        Alignment::A45,
        Alignment::A46,
        Alignment::A47,
        Alignment::A48,
        Alignment::A49,
        Alignment::A50,
        Alignment::A51,
        Alignment::A52,
        Alignment::A53,
        Alignment::A54,
        Alignment::A55,
        Alignment::A56,
        Alignment::A57,
        Alignment::A58,
        Alignment::A59,
        Alignment::A60,
        Alignment::A61,
        Alignment::A62,
        Alignment::A63,
    ];
    for (index, alignment) in alignments.into_iter().enumerate() {
        let expected = 1u64 << index;
        assert_eq!(lp1_alignment_as_u64(&alignment), expected);
        let mut copied = Alignment::A0;
        alignment_copy(&alignment, &mut copied);
        assert_eq!(copied as u64, expected);
        assert_eq!(optional_alignment(Some(alignment)), expected);
        assert_eq!(optional_alignment(None), 0);
        for choose in [false, true] {
            assert_eq!(
                alignment_phi(alignment, Alignment::A0, choose) as u64,
                if choose { expected } else { 1 }
            );
        }
        for count in [0, 1, 2, 3, 16, 17] {
            assert_eq!(
                alignment_loop(Alignment::A0, alignment, count),
                if count % 2 == 0 { 1 } else { expected }
            );
        }
        println!("direct-alignment {index} {expected}");
    }
    for value in [ByteTag::Low, ByteTag::Middle, ByteTag::High] {
        assert_eq!(byte_tag(&value), value as u64);
        println!("direct-byte {}", byte_tag(&value));
    }
    for value in [
        SignedByteTag::Low,
        SignedByteTag::Middle,
        SignedByteTag::High,
    ] {
        assert_eq!(signed_byte_tag(&value), value as i64);
        println!("direct-signed-byte {}", signed_byte_tag(&value));
    }
    for value in [SignedTag::Low, SignedTag::Middle, SignedTag::High] {
        assert_eq!(signed_tag(&value), value as i64);
        println!("direct-signed {}", signed_tag(&value));
    }
    for choose in [false, true] {
        assert_eq!(
            signed_phi(choose) as i64,
            if choose {
                i32::MIN as i64
            } else {
                i32::MAX as i64
            }
        );
    }
    for x in [0, 1, 77, u64::MAX] {
        for (value, expected) in [
            (Payload::Empty, 91),
            (Payload::Number(x), x),
            (Payload::Pair(x, 57), x ^ 57),
        ] {
            let mut copied = Payload::Empty;
            payload_copy(&value, &mut copied);
            assert_eq!(payload_value(copied), expected);
            for choose in [false, true] {
                assert_eq!(
                    payload_value(payload_phi(value, Payload::Number(11), choose)),
                    if choose { expected } else { 11 }
                );
            }
            println!("direct-payload {x} {expected}");
        }
    }
}

fn check_lp3_demand() {
    use lp3_demand_probe::*;
    fn fields(r: Record) -> (u64, u64, u64, u64) {
        (r.left.0, r.left.1, r.right.0, r.right.1)
    }
    for x in [0u64, 1, 93, u64::MAX] {
        let a = Record {
            left: (x, x ^ 17),
            right: (x.wrapping_add(5), x ^ 99),
        };
        let b = Record {
            left: (x ^ 41, x.wrapping_add(37)),
            right: (x ^ 203, x.wrapping_mul(3)),
        };
        for choose in [false, true] {
            let expected = if choose { a } else { b };
            assert_eq!(selected_field(a, b, choose), expected.right.0);
            let mut whole = expected;
            whole.left.0 ^= 99;
            assert_eq!(fields(whole_value(a, b, choose)), fields(whole));
            println!(
                "lp3-field {x} {choose} {} {:?}",
                expected.right.0,
                fields(whole)
            );
        }
        for count in [0u64, 1, 2, 9, 32] {
            let (mut left, mut right) = (a.right.0, b.right.0);
            for i in 0..count {
                (left, right) = (right.wrapping_add(i), left);
            }
            assert_eq!(sparse_loop(a, b, count), left);
            println!("lp3-loop {x} {count} {left}");
        }
        let mut source = a;
        assert_eq!(
            unsafe { snapshot_then_overwrite(&mut source, b) },
            a.left.1 ^ b.right.0
        );
        assert_eq!(fields(source), fields(b));
        source = a;
        assert_eq!(
            unsafe { repeated_snapshot(&mut source, b) },
            a.left.1.wrapping_mul(2) ^ b.right.0
        );
        assert_eq!(fields(source), fields(b));
        assert_eq!(raw_snapshot(a, b), a.right.0 ^ b.left.1);
        assert_eq!(concrete_fields(a), a.left.1.wrapping_add(a.right.0));
        assert_eq!(overwritten_field(a, x ^ 507), x ^ 507);
        let cases = [
            (Choice::Empty, 71),
            (Choice::Number(a), a.left.1),
            (Choice::Pair(a, b), b.right.0),
        ];
        for (value, expected) in cases {
            for choose in [false, true] {
                assert_eq!(
                    selected_payload(value, Choice::Number(b), choose),
                    if choose { expected } else { b.left.1 }
                );
            }
            let mut source = value;
            assert_eq!(
                unsafe { enum_snapshot_then_overwrite(&mut source, Choice::Number(b)) },
                expected
            );
            assert_eq!(selected_payload(source, Choice::Empty, true), b.left.1);
            println!("lp3-payload {x} {expected}");
        }
        println!("lp3-snapshot {x} {}", a.left.1 ^ b.right.0);
    }
}

fn check_lp3_transport() {
    use lp3_demand_probe::*;
    for seed in [0u64, 1, 3, 7, 9, 11, 15, u64::MAX] {
        let record = Record {
            left: (seed, seed.wrapping_add(1)),
            right: (seed ^ 77, seed.wrapping_mul(3)),
        };
        let mut second = record;
        second.left.0 ^= 37;
        let expected = if seed & 1 == 0 {
            Choice::Empty
        } else if seed & 2 == 0 {
            Choice::Number(record)
        } else if seed & 4 == 0 {
            Choice::Pair(record, second)
        } else {
            Choice::Pair(second, record)
        };
        let actual = transport_chain(seed);
        assert_eq!(flatten(actual), flatten(expected));
        for choose in [false, true] {
            assert_eq!(
                flatten(transport_choice(actual, Choice::Number(second), choose)),
                flatten(if choose {
                    expected
                } else {
                    Choice::Number(second)
                })
            );
        }
        println!("lp3-transport {seed} {:?}", flatten(actual));
    }
}

fn flatten(value: lp3_demand_probe::Choice) -> (u64, [u64; 8]) {
    use lp3_demand_probe::Choice;
    match value {
        Choice::Empty => (0, [0; 8]),
        Choice::Number(r) => (1, [r.left.0, r.left.1, r.right.0, r.right.1, 0, 0, 0, 0]),
        Choice::Pair(a, b) => (
            2,
            [
                a.left.0, a.left.1, a.right.0, a.right.1, b.left.0, b.left.1, b.right.0, b.right.1,
            ],
        ),
    }
}

fn check_lp3_owned_storage() {
    use lp3_demand_probe::*;
    fn record(seed: u64) -> Record {
        Record {
            left: (seed, seed ^ 17),
            right: (seed.wrapping_add(5), seed.wrapping_mul(3)),
        }
    }
    fn first_value(seed: u64) -> Choice {
        let r = record(seed);
        if seed & 1 == 0 {
            Choice::Number(r)
        } else {
            Choice::Pair(r, r)
        }
    }
    for seed in [0u64, 1, 2, 3, 17, 99, u64::MAX - 1, u64::MAX] {
        let first = first_value(seed);
        let mut changed = record(seed);
        changed.right.0 ^= 203;
        let second = if seed & 2 == 0 {
            Choice::Pair(changed, record(seed))
        } else {
            Choice::Number(changed)
        };
        for choose in [false, true] {
            let (mut a, mut b, mut c) = (Choice::Empty, Choice::Empty, Choice::Empty);
            unsafe { transport_multiple_sinks(seed, choose, &mut a, &mut b, &mut c) };
            assert_eq!(flatten(a), flatten(first));
            assert_eq!(flatten(b), flatten(second));
            assert_eq!(flatten(c), flatten(if choose { first } else { second }));
            // Overwriting the same external destination must not change the
            // private first snapshot selected again by the final store.
            let mut shared = Choice::Empty;
            let ptr = &mut shared as *mut Choice;
            unsafe { transport_multiple_sinks(seed, choose, ptr, ptr, ptr) };
            assert_eq!(
                flatten(shared),
                flatten(if choose { first } else { second })
            );
        }
        for count in [0u64, 1, 2, 7] {
            let mut output = Choice::Empty;
            unsafe { transport_inside_loop(seed, count, &mut output) };
            let expected = if count == 0 {
                Choice::Empty
            } else {
                first_value(seed.wrapping_add(count - 1))
            };
            assert_eq!(flatten(output), flatten(expected));
        }
        println!("lp3-owned-storage {seed}");
    }
}

fn check_lp3_branch_snapshots() {
    use lp3_demand_probe::*;
    for value in [0u64, 1, 17, 99, 255, 1024, u64::MAX - 1, u64::MAX] {
        for a in [false, true] {
            for b in [false, true] {
                let mut memory = 123;
                let stored = unsafe { branch_stored_snapshot(&mut memory, value, a, b) };
                let expected = if a {
                    if b {
                        value.wrapping_add(17)
                    } else {
                        value ^ 99
                    }
                } else {
                    71
                };
                assert_eq!(stored, expected);
                assert_eq!(memory, value);
                let loaded = unsafe { branch_loaded_snapshot(&mut memory, a, b) };
                let expected = if a {
                    if b { value.wrapping_mul(2) } else { 0 }
                } else {
                    value.wrapping_sub(3)
                };
                assert_eq!(loaded, expected);
                assert_eq!(memory, value);
            }
            let replacement = value ^ 507;
            let mut memory = 123;
            let pointer = &mut memory as *mut u64;
            assert_eq!(
                unsafe { branch_alias_write(pointer, pointer, value, replacement, a) },
                if a { replacement } else { value ^ 99 }
            );
            assert_eq!(memory, if a { replacement } else { value });
            let mut distinct = 37;
            assert_eq!(
                unsafe { branch_alias_write(pointer, &mut distinct, value, replacement, a) },
                if a { value } else { value ^ 99 }
            );
            assert_eq!(memory, value);
            assert_eq!(distinct, if a { replacement } else { 37 });
        }
        println!("lp3-branch-snapshot {value}");
    }
}

fn check_lp3_nested_transport() {
    use lp3_demand_probe::*;
    fn problem(value: Problem) -> (u64, u64, u64) {
        match value {
            Problem::Missing => (0, 0, 0),
            Problem::Character(c) => (1, c as u64, 0),
            Problem::Pair(a, b) => (2, a, b),
        }
    }
    fn fields(value: Record) -> [u64; 4] {
        [value.left.0, value.left.1, value.right.0, value.right.1]
    }
    for seed in [0u64, 1, 17, u64::MAX] {
        let original = Record {
            left: (seed, seed ^ 17),
            right: (seed.wrapping_add(5), seed.wrapping_mul(3)),
        };
        let replacement = Record {
            left: (37, 41),
            right: (43, 47),
        };
        for selector in 0..4 {
            let mut memory = original;
            let expected_problem = match selector {
                1 => Problem::Missing,
                2 => Problem::Character('界'),
                _ => Problem::Pair(original.left.0, original.right.1),
            };
            match unsafe { nested_result_transport(&mut memory, selector, replacement) } {
                Ok(value) => {
                    assert_eq!(selector, 0);
                    assert_eq!(
                        fields(value),
                        [original.right.0, 17, 99, original.right.0 ^ 203]
                    );
                }
                Err(value) => {
                    assert_ne!(selector, 0);
                    assert_eq!(problem(value), problem(expected_problem));
                }
            }
            assert_eq!(fields(memory), fields(replacement));
            let (observed, value) = unsafe { nested_result_observed(&original, selector) };
            let expected_observed = match selector {
                0 => original.right.0,
                1 => 71,
                2 => '界' as u64,
                _ => original.left.0 ^ original.right.1,
            };
            assert_eq!(observed, expected_observed);
            assert_eq!(
                problem(value),
                problem(if selector == 0 {
                    Problem::Missing
                } else {
                    expected_problem
                })
            );
            println!("lp3-nested-transport {seed} {selector} {observed}");
        }
    }
}
