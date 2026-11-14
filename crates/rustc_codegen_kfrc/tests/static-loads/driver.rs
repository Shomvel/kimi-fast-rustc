extern crate static_loads as subject;

fn main() {
    let mut checksum = 0u64;
    let expected = [1u8, 0, 0x7f, 0x80, b'=', 0xff, 37, 9];
    let mut previous = 17;
    for value in 0..=u16::MAX {
        assert_eq!(subject::constant_byte(), b'=');
        assert_eq!(subject::constant_signed(), i16::MIN);
        assert_eq!(subject::indirect(), 91);
        assert_eq!(subject::relocated_chain(), b'=');
        assert_ne!(subject::relocated_address(), 0);
        assert!(subject::offset_address_matches());
        let index = usize::from(value);
        assert_eq!(subject::indexed(index), expected[index & 7]);
        let byte = value as u8;
        assert_eq!(subject::equals(Some(&byte)), byte == b'=');
        assert!(!subject::equals(None));
        let current = (u64::from(value) << 32) | u64::from(!value);
        assert_eq!(unsafe { subject::writable(current) }, previous);
        previous = current;
        checksum = checksum.wrapping_add(u64::from(subject::indexed(index)));
    }
    println!("static-load differential {checksum}");
}
