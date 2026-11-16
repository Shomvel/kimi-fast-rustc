use scalar_abi_candidate as candidate;
use scalar_abi_foreign as foreign;
use std::hint::black_box;

fn record(checks: &mut u64, digest: &mut u64, actual: u64, expected: u64) {
    assert_eq!(actual, expected, "boundary check {}", *checks);
    *checks += 1;
    *digest = digest.rotate_left(7).wrapping_add(actual);
}

fn main() {
    let (mut checks, mut digest) = (0, 0);
    if std::env::args().any(|argument| argument == "--wrapper-extension") {
        for raw in 0..=u8::MAX {
            let signed = raw as i8;
            record(
                &mut checks,
                &mut digest,
                candidate::checked_wrapper(foreign::SignedByte(signed)),
                signed as i64 as u64,
            );
        }
        println!("signed wrapper extension checks={checks} checksum={digest:016x}");
        return;
    }
    macro_rules! test_scalar {
        ($module:ident, $value:expr, $check:ident, $poison_c:ident, $poison_rust:ident) => {{
            let value = $value;
            let expected = foreign::$module::rust(value) as i64 as u64;
            let raw = value as i64 as u64;
            let rust = black_box(foreign::$module::rust);
            let c = black_box(foreign::$module::c);
            for actual in [
                candidate::$module::rust(value) as i64 as u64,
                candidate::$module::c(value) as i64 as u64,
                candidate::$module::direct_rust(value),
                candidate::$module::direct_c(value),
                candidate::$module::indirect_rust(rust, value),
                candidate::$module::indirect_c(c, value),
            ] {
                record(&mut checks, &mut digest, actual, expected);
            }
            for actual in [
                candidate::$module::checked_argument(value),
                candidate::$module::dirty_c_result(raw),
                candidate::$module::dirty_rust_result(raw),
                unsafe { foreign::$module::$check(value) },
                unsafe { foreign::$module::$poison_c(candidate::$module::echo_c, raw) },
                unsafe { foreign::$module::$poison_rust(candidate::$module::echo_rust, raw) },
            ] {
                record(&mut checks, &mut digest, actual, raw);
            }
        }};
    }
    for raw in 0..2 {
        test_scalar!(
            boolean,
            raw != 0,
            check_c_boolean,
            poison_c_boolean,
            poison_rust_boolean
        );
    }
    for raw in 0..=u8::MAX {
        test_scalar!(
            unsigned8,
            raw,
            check_c_unsigned8,
            poison_c_unsigned8,
            poison_rust_unsigned8
        );
        test_scalar!(
            signed8,
            raw as i8,
            check_c_signed8,
            poison_c_signed8,
            poison_rust_signed8
        );
        let expected = (raw as i8).wrapping_add(11);
        record(
            &mut checks,
            &mut digest,
            candidate::wrapper(foreign::SignedByte(raw as i8)).0 as i64 as u64,
            expected as i64 as u64,
        );
        record(
            &mut checks,
            &mut digest,
            candidate::c_wrapper(foreign::SignedByte(raw as i8)).0 as i64 as u64,
            expected as i64 as u64,
        );
        let mut bytes = [0xA5, 0xCC, 0x5A];
        record(
            &mut checks,
            &mut digest,
            candidate::write_byte(&mut bytes, raw) as u64,
            raw as u64,
        );
        assert_eq!(bytes, [0xA5, raw, 0x5A]);
    }
    for raw in 0..=u16::MAX {
        test_scalar!(
            unsigned16,
            raw,
            check_c_unsigned16,
            poison_c_unsigned16,
            poison_rust_unsigned16
        );
        test_scalar!(
            signed16,
            raw as i16,
            check_c_signed16,
            poison_c_signed16,
            poison_rust_signed16
        );
        let flag = raw & 1 != 0;
        let value = foreign::Pair {
            number: raw as i16,
            predicate: flag,
        };
        let bias = raw.rotate_left(3) as i16;
        let expected = foreign::pair(value, bias);
        for actual in [
            candidate::pair(value, bias),
            candidate::direct_pair(value, bias),
            candidate::indirect_pair(black_box(foreign::pair), value, bias),
        ] {
            record(
                &mut checks,
                &mut digest,
                actual.number as i64 as u64,
                expected.number as i64 as u64,
            );
            record(
                &mut checks,
                &mut digest,
                actual.predicate as u64,
                expected.predicate as u64,
            );
        }
        let raw_signed = value.number as i64 as u64;
        let expected = raw_signed.wrapping_add(flag as u64);
        record(
            &mut checks,
            &mut digest,
            candidate::dirty_pair(raw_signed, flag),
            expected,
        );
        record(
            &mut checks,
            &mut digest,
            candidate::checked_pair(value),
            expected,
        );
        let expected = (63 ^ raw_signed).wrapping_add(flag as u64);
        record(
            &mut checks,
            &mut digest,
            unsafe { foreign::poison_stack_rust(candidate::stack_rust, raw_signed, flag as u64) },
            expected,
        );
        record(
            &mut checks,
            &mut digest,
            unsafe { foreign::poison_stack_c(candidate::stack_c, raw_signed, flag as u64) },
            expected,
        );
    }
    let mut state = 0x1234_5678_u32;
    for index in 0..4096 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let raw = match index {
            0 => 0,
            1 => 1,
            2 => u32::MAX,
            3 => i32::MAX as u32,
            4 => i32::MIN as u32,
            5 => 0xFFFF,
            6 => 0x1_0000,
            _ => state,
        };
        test_scalar!(
            unsigned32,
            raw,
            check_c_unsigned32,
            poison_c_unsigned32,
            poison_rust_unsigned32
        );
        test_scalar!(
            signed32,
            raw as i32,
            check_c_signed32,
            poison_c_signed32,
            poison_rust_signed32
        );
    }
    println!("scalar ABI matrix checks={checks} checksum={digest:016x}");
}
