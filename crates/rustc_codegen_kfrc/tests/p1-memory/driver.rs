#![feature(simd_ffi)]

extern crate memory;

mod pointer_comparison_driver;
mod simd_driver;

fn selection_probes() {
    for (left, right) in [(0_i64, 7_i64), (-1, 23), (i64::MIN, i64::MAX)] {
        for condition in [false, true] {
            let expected = if condition { left } else { right };
            let scalars = memory::select::scalars(condition, left as u64, right as u64);
            assert_eq!(
                scalars,
                (
                    expected as u64,
                    !condition,
                    if condition { 'λ' } else { '中' },
                    true
                )
            );
            println!("select-scalars {condition} {left} {right} {scalars:?}");
            let aggregates = memory::select::aggregates(condition, left, right);
            assert_eq!(
                aggregates,
                (
                    expected,
                    condition,
                    expected.wrapping_add(if condition { 3 } else { 7 }),
                    expected as u64,
                    expected.wrapping_add(if condition { 0 } else { 11 }),
                    condition,
                )
            );
            println!("select-aggregates {condition} {left} {right} {aggregates:?}");
            let copies = memory::select::copies(condition, left, right);
            let delta = if condition { 1_i64 } else { -1_i64 };
            assert_eq!(
                copies,
                (
                    [
                        expected,
                        expected.wrapping_add(delta),
                        expected.wrapping_add(delta * 2)
                    ],
                    1665
                )
            );
            println!("select-copies {condition} {left} {right} {copies:?}");
            let owned = memory::select::owned(condition, left, right);
            assert_eq!(owned, (expected, 1, 2));
            println!("select-owned {condition} {left} {right} {owned:?}");
            let intrinsic_owned = memory::select::intrinsic_owned(condition, left, right);
            assert_eq!(intrinsic_owned, (expected, 0, 1));
            println!("select-intrinsic-owned {condition} {left} {right} {intrinsic_owned:?}");
            let eager = memory::select::eager_arguments(condition);
            assert_eq!(eager, (if condition { 1 } else { 2 }, 12));
            println!("select-eager {condition} {left} {right} {eager:?}");
            let wide = memory::select::wide_pointer(condition);
            assert_eq!(wide, if condition { (5, b'a') } else { (4, b'b') });
            println!("select-wide {condition} {left} {right} {wide:?}");
            let zero = memory::select::zero_sized(condition);
            assert_eq!(zero, 4);
            println!("select-zero {condition} {left} {right} {zero}");
            let zero_owned = memory::select::owned_zero_sized(condition);
            assert_eq!(zero_owned, (1, 2));
            println!("select-zero-owned {condition} {left} {right} {zero_owned:?}");
        }
    }
}

fn thread_local_probes() {
    use std::sync::{Arc, Barrier};
    fn expected(seed: u64) -> (u64, u64, u64, u64, u64, u64) {
        (
            seed.wrapping_add(5),
            seed.wrapping_mul(3),
            seed.wrapping_add(7),
            seed.wrapping_mul(11),
            seed.wrapping_add(13),
            seed.wrapping_mul(17),
        )
    }
    assert_eq!(memory::thread_local::immutable_value(), 17);
    assert_eq!(memory::thread_local::read(), (23, 0, 29, 31, 37, 41));
    memory::thread_local::write(100);
    assert_eq!(memory::thread_local::read(), expected(100));
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    let mut markers = Vec::new();
    for index in 0..2 {
        let mut marker = Box::new([0_u64; 2]);
        let marker_address = marker.as_mut_ptr() as usize;
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            let seed = 200 + index * 100;
            let initial_ok = memory::thread_local::immutable_value() == 17
                && memory::thread_local::read() == (23, 0, 29, 31, 37, 41);
            unsafe { memory::thread_local::set_exit_marker(marker_address as *mut u64, index + 1) };
            memory::thread_local::write(seed);
            barrier.wait();
            let persistent_ok = memory::thread_local::read() == expected(seed);
            memory::thread_local::write(seed + 1);
            barrier.wait();
            let state = memory::thread_local::read();
            assert!(initial_ok);
            assert!(persistent_ok);
            assert_eq!(state, expected(seed + 1));
            let random = memory::thread_local::lazy_random_state(seed);
            assert_eq!(random, seed * 4 + 21);
            (state, random)
        }));
        markers.push(marker);
    }
    barrier.wait();
    let main_isolated = memory::thread_local::read() == expected(100);
    barrier.wait();
    assert!(main_isolated);
    for (index, handle) in handles.into_iter().enumerate() {
        let result = handle.join().unwrap();
        assert_eq!(*markers[index], [index as u64 + 1, (index as u64 + 1) * 10]);
        println!("tls-thread {index} {result:?} drop {:?}", *markers[index]);
    }
    let final_state = memory::thread_local::read();
    assert_eq!(final_state, expected(100));
    println!(
        "tls-main {final_state:?} random {}",
        memory::thread_local::lazy_random_state(100)
    );
}

fn bit_operation_probes() {
    macro_rules! check {
        ($integer:ty, $rotate:ident, $funnel:ident) => {{
            let width = <$integer>::BITS;
            for (left, right) in [
                (0, 0),
                (0, !0),
                (!0, 0),
                (
                    0x9b73_05a1_f2c8_4e61_u64 as $integer,
                    0x63d8_b20f_19a5_c472_u64 as $integer,
                ),
                (1, !1),
            ] {
                for shift in [0, 1, width - 1] {
                    println!(
                        "funnel {} {left} {right} {shift} {:?}",
                        stringify!($integer),
                        memory::bitops::$funnel(left, right, shift)
                    );
                }
                for shift in [0, 1, width - 1, width, width + 1, 2 * width - 1, u32::MAX] {
                    println!(
                        "rotate {} {left} {shift} {:?}",
                        stringify!($integer),
                        memory::bitops::$rotate(left, shift)
                    );
                }
            }
        }};
    }
    check!(u8, rotate_u8, funnel_u8);
    check!(u16, rotate_u16, funnel_u16);
    check!(u32, rotate_u32, funnel_u32);
    check!(u64, rotate_u64, funnel_u64);
    check!(usize, rotate_usize, funnel_usize);
    check!(i8, rotate_i8, funnel_i8);
    check!(i16, rotate_i16, funnel_i16);
    check!(i32, rotate_i32, funnel_i32);
    check!(i64, rotate_i64, funnel_i64);
    check!(isize, rotate_isize, funnel_isize);
}

fn saturation_probes() {
    macro_rules! check_pair {
        ($integer:ty, $function:ident, $left:expr, $right:expr) => {{
            let left: $integer = $left;
            let right: $integer = $right;
            // The driver uses LLVM and a wider mathematical reference. The
            // kfrc library exercises both the intrinsic and primitive methods.
            let minimum = <$integer>::MIN as i128;
            let maximum = <$integer>::MAX as i128;
            let add = (left as i128 + right as i128).clamp(minimum, maximum) as $integer;
            let sub = (left as i128 - right as i128).clamp(minimum, maximum) as $integer;
            assert_eq!(
                memory::saturation::$function(left, right),
                (add, sub, add, sub),
                "saturation {} {left} {right}",
                stringify!($integer),
            );
        }};
    }
    for left in 0..=u8::MAX as u16 {
        for right in 0..=u8::MAX as u16 {
            check_pair!(u8, saturate_u8, left as u8, right as u8);
            check_pair!(i8, saturate_i8, left as i8, right as i8);
        }
    }
    println!("saturation u8/i8: all 131072 operand pairs verified");
    macro_rules! boundaries {
        ($integer:ty, $function:ident) => {{
            let minimum = <$integer>::MIN;
            let maximum = <$integer>::MAX;
            let values: [$integer; 10] = [
                minimum,
                minimum.wrapping_add(1),
                minimum.wrapping_add(2),
                (-1i128) as $integer,
                0,
                1,
                2,
                maximum.wrapping_sub(2),
                maximum.wrapping_sub(1),
                maximum,
            ];
            for left in values {
                for right in values {
                    check_pair!($integer, $function, left, right);
                }
            }
            println!(
                "saturation {}: 100 boundary pairs verified",
                stringify!($integer)
            );
        }};
    }
    boundaries!(u16, saturate_u16);
    boundaries!(u32, saturate_u32);
    boundaries!(u64, saturate_u64);
    boundaries!(usize, saturate_usize);
    boundaries!(i16, saturate_i16);
    boundaries!(i32, saturate_i32);
    boundaries!(i64, saturate_i64);
    boundaries!(isize, saturate_isize);
}

fn main() {
    if let Some(mode) = std::env::args().nth(1) {
        match mode.as_str() {
            "division" => {
                std::hint::black_box(memory::division(123, 0));
            }
            "bounds" => {
                std::hint::black_box(memory::array_index(7, 4));
            }
            "overflow" => {
                std::hint::black_box(memory::checked(i32::MAX, 1));
            }
            _ => panic!("unexpected mode"),
        }
        return;
    }
    for a in [-137, -9, -1, 0, 1, 7, 42, 1000] {
        println!("guard {} {}", a, memory::guard_loop(a));
        println!(
            "thin-metadata {} {}",
            a,
            memory::pointer_metadata::thin(a as u64)
        );
        println!("pair {} {}", a, memory::pair_use(a));
        println!("pair-arg {} {}", a, memory::pair_argument_use(a));
        println!("enum {} {}", a, memory::explicit_enum(a));
        println!("niche {} {}", a, memory::niche_reference(a));
        println!("option {} {}", a, memory::tagged_option(a));
        println!("slice {} {}", a, memory::slice_use(a));
        println!("raw-slice {} {}", a, memory::raw_slice_use(a));
        println!("black-box {} {}", a, memory::black_box_value(a));
        println!("subslice {} {}", a, memory::subslice(a));
        println!("disabled-check {} {}", a, memory::disabled_check_paths(a));
        println!(
            "trivial-drop {} {}",
            a,
            memory::trivial_drop_paths(a as u64)
        );
        println!("closure-fn {} {}", a, memory::reified_closure(a));
        println!("dyn-call {} {}", a, memory::trait_object_use(a));
        println!("dyn-drop {} {}", a, memory::trait_object_drop(a));
        println!(
            "dyn-noop-drop {} {}",
            a,
            memory::trait_object_trivial_drop(a)
        );
        println!("boxed-slice {} {}", a, memory::boxed_slice(a));
        println!("boxed-dyn {} {}", a, memory::boxed_trait_object(a));
        println!("tagged-boxed-slice {} {}", a, memory::tagged_boxed_slice(a));
        println!("struct-tail-slice {} {}", a, memory::struct_tail_slice(a));
        println!("fn-items {} {}", a, memory::function_item_callbacks(a));
        println!(
            "fn-pointers {} {}",
            a,
            memory::function_pointer_callbacks(a)
        );
        println!("captured-callbacks {} {}", a, memory::captured_callbacks(a));
        println!(
            "tracked-callbacks {} {}",
            a,
            memory::tracked_function_callbacks(a)
        );
        println!(
            "virtual-method-item {} {}",
            a,
            memory::virtual_method_item(a)
        );
        println!(
            "rustcall-constants {} {}",
            a,
            memory::rustcall::constant_arguments(a)
        );
        println!(
            "boxed-once-args {} {}",
            a,
            memory::rustcall::boxed_once_arguments(a)
        );
        println!(
            "boxed-once-zst {} {}",
            a,
            memory::rustcall::boxed_once_zero_sized()
        );
        println!(
            "boxed-method {} {}",
            a,
            memory::rustcall::boxed_method_receiver(a)
        );
        println!(
            "union-fields {} {}",
            a,
            memory::unions::aggregate_fields(a as u64)
        );
        println!(
            "union-maybe-uninit {} {}",
            a,
            memory::unions::maybe_uninit_and_zero_sized(a)
        );
        println!(
            "zero-sized-drops {} {}",
            a,
            memory::zero_sized::aggregate_drops()
        );
        println!(
            "stack-alignment {} {:?}",
            a,
            memory::stack_alignment::locals(a as u64)
        );
        println!(
            "capture-alignment {} {:?}",
            a,
            memory::stack_alignment::captured(a as u64)
        );
        println!(
            "static-scalar {} {}",
            a,
            memory::static_data::scalar_storage(a as u64)
        );
        println!(
            "static-pointer {} {}",
            a,
            memory::static_data::pointer_storage(a as u64)
        );
        println!(
            "static-interior {} {}",
            a,
            memory::static_data::interior_storage(a as u64)
        );
        println!(
            "static-immutable {} {}",
            a,
            memory::static_data::immutable_storage(a as usize)
        );
        for b in [-23, -1, 1, 9, 127] {
            println!("cmp {a} {b} {}", memory::ordering(a, b));
            println!("ref {a} {b} {}", memory::references(a, b));
            println!("tuple {a} {b} {}", memory::tuple_fields(a, b));
            println!("div {a} {b} {}", memory::division(a, b));
            println!("checked {a} {b} {}", memory::checked(a as i32, b as i32));
        }
        for index in 0..4 {
            println!("array {a} {index} {}", memory::array_index(a, index));
        }
    }
    for index in 0..8 {
        println!("const {index} {}", memory::const_bytes(index));
    }
    println!("str {} {}", memory::string_length(), memory::string_byte());
    println!("track-caller-fn {:?}", memory::reified_tracked_location());
    bit_operation_probes();
    saturation_probes();
    selection_probes();
    thread_local_probes();
    simd_driver::run();
    pointer_comparison_driver::run();
    for (left, right) in [(0, 7), (-1, 23), (i64::MIN, i64::MAX)] {
        println!(
            "known-hint {left} {:?}",
            memory::const_hints::runtime_hints(left)
        );
        println!(
            "discarded-known-hint {left} {}",
            memory::const_hints::discarded_hint(left)
        );
        println!(
            "const-runtime-select {left} {:?}",
            memory::const_hints::const_and_runtime(left)
        );
        println!(
            "literal-known-hint {left} {}",
            memory::const_hints::literal_hint(left)
        );
        println!(
            "nonenum-discriminants {left} {:?}",
            memory::intrinsic_values::discriminants(left)
        );
        println!(
            "typed-swap-scalar {left} {right} {:?}",
            memory::intrinsic_values::scalar_swap(left, right)
        );
        println!(
            "typed-swap-aggregate {left} {right} {:?}",
            memory::intrinsic_values::aggregate_swap(left, right)
        );
        println!(
            "typed-swap-union-niche {left} {right} {:?}",
            memory::intrinsic_values::union_niche_swap(left as u64, right as u64)
        );
        println!(
            "typed-swap-owned-zst {left} {right} {:?}",
            memory::intrinsic_values::owned_zst_swap(left, right)
        );
    }
}
