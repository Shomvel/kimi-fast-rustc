extern crate program_left;
extern crate program_right;
extern crate program_shared;

fn main() {
    assert!(program_shared::identities());
    let mut checksum = 0u64;
    let mut previous = 19;
    let left_generic = program_left::generic_address();
    let right_generic = program_right::generic_address();
    for index in 0..4096u64 {
        let value = index.wrapping_mul(0x5851f42d4c957f2d);
        let generic = value
            .wrapping_mul(0x9e3779b97f4a7c15)
            .rotate_left(7)
            .wrapping_add(29);
        assert_eq!(left_generic(value), generic);
        assert_eq!(right_generic(value), generic);
        let left = program_left::evaluate(value);
        let right = program_right::evaluate(value);
        assert_eq!(left, right);
        checksum = checksum.rotate_left(3) ^ left;
        assert_eq!(program_left::exchange(value), previous);
        assert_eq!(program_right::exchange(value.wrapping_add(1)), value);
        previous = value.wrapping_add(1);
    }
    // Optional future TLS frontend probe, compiled with --cfg=program_tls.
    #[cfg(program_tls)]
    {
        assert_eq!(program_left::tls_read(), 37);
        program_left::tls_write(101);
        assert_eq!(program_right::tls_read(), 101);
        for value in [211, 307] {
            std::thread::spawn(move || {
                assert_eq!(program_right::tls_read(), 37);
                program_right::tls_write(value);
                assert_eq!(program_left::tls_read(), value);
            })
            .join()
            .unwrap();
        }
        assert_eq!(program_right::tls_read(), 101);
    }
    println!("program-data: {checksum:016x}; identities, shared state and rlib linkage agree");
}
