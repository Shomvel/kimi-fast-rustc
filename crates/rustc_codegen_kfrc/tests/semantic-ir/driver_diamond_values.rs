use semantic_values::diamond_values_probe::*;

pub fn check() {
    let mut trace = 0u64;
    let mut expected_trace = 0u64;
    for value in 0..=u8::MAX {
        assert_eq!(increment_or_zero(value), value.wrapping_add(1));
        assert_eq!(decrement_or_max(value), value.wrapping_sub(1));
        let expression = value.wrapping_mul(3).wrapping_add(1);
        assert_eq!(equivalent_constant(value), expression);
        assert_eq!(
            different_constant(value),
            if value == 37 { 113 } else { expression }
        );
        assert_eq!(nonunit_increment(value), value.checked_add(2).unwrap_or(0));
        if value == u8::MAX {
            expected_trace = expected_trace.wrapping_add(7);
        }
        assert_eq!(observed_increment(value, &mut trace), value.wrapping_add(1));
        assert_eq!(trace, expected_trace);
    }
    let mut samples = vec![0, 1, 2, i64::MAX as u64, 1u64 << 63, u64::MAX - 1, u64::MAX];
    let mut random = 0xf8e7_d6c5_b4a3_9281u64;
    for _ in 0..1024 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        samples.push(random);
    }
    for &value in &samples {
        assert_eq!(word_increment(value), value.wrapping_add(1));
        assert_eq!(
            signed_equality(value as i64),
            (value as i64).wrapping_add(1)
        );
        assert_eq!(
            signed_guarded_unchecked(value as i64),
            (value as i64).wrapping_add(1)
        );
    }
    println!(
        "diamond-values bytes=256 words={} trace={trace}",
        samples.len()
    );
}
