extern crate abi_reference;
extern crate semantic_values;
use abi_reference::{Big, Pair};
use semantic_values::*;

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
