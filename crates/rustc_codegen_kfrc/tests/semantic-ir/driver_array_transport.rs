use semantic_values::array_transport_probe::*;

pub fn check() {
    let mut checksum = 0u64;
    for seed in [0u8, 1, 127, 255] {
        let mut source = make(seed);
        let expected = std::array::from_fn::<_, 256, _>(|i| seed.wrapping_add((i as u8).wrapping_mul(7)));
        assert_eq!(source, expected);
        let captured = copy_then_mutate(&mut source);
        assert_eq!(captured, expected);
        assert_eq!(source, [99; 256]);
        let packet = wrap(seed);
        let expected = std::array::from_fn::<_, 64, _>(|i| seed.wrapping_add(i as u8));
        assert_eq!(packet.head, seed);
        assert_eq!(packet.tail, 0x1234_5678_9abc_def0);
        assert_eq!(packet.table, expected);
        assert_eq!(observe(packet.table), expected[7] ^ expected[61]);
        for count in 0..5 {
            assert_eq!(array_phi(packet.table, count), if count % 2 == 0 { expected } else { [17; 64] });
        }
        for index in [0, 7, 63] {
            let result = update(packet.table, index, 193);
            let mut wanted = expected;
            wanted[index] = 193;
            assert_eq!(result, wanted);
            checksum += result.iter().map(|&x| u64::from(x)).sum::<u64>();
        }
        checksum += captured.iter().map(|&x| u64::from(x)).sum::<u64>();
    }
    println!("array-transport {checksum}");
}
