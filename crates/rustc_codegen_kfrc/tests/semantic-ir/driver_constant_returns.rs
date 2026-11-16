use semantic_values::constant_returns_probe::*;

pub fn check() {
    assert_eq!(
        [u8_low() as u64, u8_high() as u64, u8_middle() as u64],
        [0, 255, 128]
    );
    assert_eq!(
        [i8_low() as i64, i8_high() as i64, i8_middle() as i64],
        [-128, 127, -1]
    );
    assert_eq!(
        [u16_low() as u64, u16_high() as u64, u16_middle() as u64],
        [0, 65535, 32768]
    );
    assert_eq!(
        [i16_low() as i64, i16_high() as i64, i16_middle() as i64],
        [-32768, 32767, -1]
    );
    assert_eq!(
        [u32_low() as u64, u32_high() as u64, u32_middle() as u64],
        [0, u32::MAX as u64, 1 << 31]
    );
    assert_eq!(
        [i32_low() as i64, i32_high() as i64, i32_middle() as i64],
        [i32::MIN as i64, i32::MAX as i64, -1]
    );
    assert!(yes());
    assert!(!no());
    let mut saved = 0;
    assert_eq!(shared(&mut saved), -1);
    assert_eq!(saved, -1);
    assert_eq!(pair(), (-128, 0x8000));
    for value in 0..=u16::MAX {
        assert_eq!(dynamic_signed(value as i8) as i64, (value as i8) as i64);
        assert_eq!(dynamic_unsigned(value) as u64, value as u64);
        assert_eq!(mixed(value as i8), (0x8000, value as i8));
    }
    println!("constant-return ABI lanes and 65536 dynamic values agree");
}
