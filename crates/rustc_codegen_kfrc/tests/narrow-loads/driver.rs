extern crate narrow_loads as subject;

fn main() {
    let mut checksum = 0u64;
    for value in 0..=u16::MAX {
        let byte = value as u8;
        let word = (u32::from(value) << 16) | u32::from(!value);
        let mut saved_byte = 0;
        let mut saved_half = 0;
        let mut saved_word = 0;
        let mut saved_signed = 0;
        let actual = subject::byte(&byte, &mut saved_byte);
        assert_eq!((actual, saved_byte), (u64::from(byte), byte));
        checksum = checksum.wrapping_add(actual);
        let actual = subject::half(&value, &mut saved_half);
        assert_eq!((actual, saved_half), (u64::from(value), value));
        checksum = checksum.wrapping_add(actual);
        let actual = subject::word(&word, &mut saved_word);
        assert_eq!((actual, saved_word), (u64::from(word), word));
        checksum = checksum.wrapping_add(actual);
        let signed = byte as i8;
        let actual = subject::signed(&signed, &mut saved_signed);
        assert_eq!(
            (actual, saved_signed),
            (u64::from((signed as i16) as u16), signed as i16)
        );
        checksum = checksum.wrapping_add(actual);
        let actual = subject::increment_byte(&byte, &mut saved_byte);
        assert_eq!(
            (actual, saved_byte),
            (u64::from(byte.wrapping_add(1)), byte.wrapping_add(1))
        );
        checksum = checksum.wrapping_add(actual);
        let actual = subject::increment_half(&value, &mut saved_half);
        assert_eq!(
            (actual, saved_half),
            (u64::from(value.wrapping_add(1)), value.wrapping_add(1))
        );
        checksum = checksum.wrapping_add(actual);
        assert_eq!(subject::parameter(byte), u64::from(byte.wrapping_add(1)));
    }
    println!("narrow-load differential {checksum}");
}
