#![allow(unused, non_snake_case, unconditional_panic)]
// tests/integer_ops.rs::variable_negation_wraps_at_the_i64_boundary
pub mod case_000 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return -value; }
}
// tests/integer_ops.rs::bitwise_precedence_and_comparisons_match_the_interpreter
pub mod case_001 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> i64 { return a | b ^ a & b; }
}
// tests/integer_ops.rs::bitwise_precedence_and_comparisons_match_the_interpreter
pub mod case_002 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> i64 { return a + b << 2; }
}
// tests/integer_ops.rs::bitwise_precedence_and_comparisons_match_the_interpreter
pub mod case_003 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> bool { return a | b == 7; }
}
// tests/integer_ops.rs::every_bitwise_operator_matches_the_interpreter
pub mod case_004 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> i64 { return a & b; }
}
// tests/integer_ops.rs::every_bitwise_operator_matches_the_interpreter
pub mod case_005 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> i64 { return a | b; }
}
// tests/integer_ops.rs::every_bitwise_operator_matches_the_interpreter
pub mod case_006 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> i64 { return a ^ b; }
}
// tests/integer_ops.rs::shifts_mask_counts_and_preserve_the_sign
pub mod case_007 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> i64 { return a << b; }
}
// tests/integer_ops.rs::shifts_mask_counts_and_preserve_the_sign
pub mod case_008 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> i64 { return a >> b; }
}
// tests/integer_ops.rs::shifts_survive_spills_and_loop_phi_values
pub mod case_009 {
#[inline(never)] pub fn test_fn(a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64, h: i64, i: i64, j: i64, k: i64, l: i64) -> i64 { let mut value = 3; let mut count = 0; while count < 4 { value = value << 2; count = count + 1; } return (value >> 1) + a + b + c + d + e + f + g + h + i + j + k + l; }
}
// tests/integer_not.rs::signed_integer_not_matches_bitwise_complement
pub mod case_010 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return !value; }
}
// tests/integer_not.rs::unsigned_integer_not_preserves_the_full_bit_pattern
pub mod case_011 {
#[inline(never)] pub fn test_fn(value: u64) -> u64 { return !value; }
}
// tests/strength_reduction.rs::multiplication_by_power_of_two_becomes_left_shift
pub mod case_012 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return value * 8; }
}
// tests/strength_reduction.rs::power_of_two_on_the_left_also_becomes_left_shift
pub mod case_013 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return 8 * value; }
}
// tests/strength_reduction.rs::unsigned_division_by_power_of_two_becomes_logical_shift
pub mod case_014 {
#[inline(never)] pub fn test_fn(value: u64) -> u64 { return value / 8; }
}
// tests/strength_reduction.rs::signed_power_division_uses_shifts_and_non_power_multiplication_is_unchanged
pub mod case_015 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return value / 8; }
}
// tests/strength_reduction.rs::signed_power_division_uses_shifts_and_non_power_multiplication_is_unchanged
pub mod case_016 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return value * 3; }
}
// tests/unary_strength.rs::zero_minus_becomes_wrapping_negation
pub mod case_017 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return 0 - value; }
}
// tests/unary_strength.rs::multiplication_by_minus_one_becomes_negation_on_either_side
pub mod case_018 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return -1 * value; }
}
// tests/unary_strength.rs::multiplication_by_minus_one_becomes_negation_on_either_side
pub mod case_019 {
#[inline(never)] pub fn test_fn(value: u64) -> u64 { return value * 0xffff_ffff_ffff_ffffu64; }
}
// tests/unary_strength.rs::xor_with_all_ones_becomes_bitwise_not_on_either_side
pub mod case_020 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return -1 ^ value; }
}
// tests/unary_strength.rs::xor_with_all_ones_becomes_bitwise_not_on_either_side
pub mod case_021 {
#[inline(never)] pub fn test_fn(value: u64) -> u64 { return value ^ 0xffff_ffff_ffff_ffffu64; }
}
// tests/unsigned_remainder.rs::unsigned_remainder_by_a_power_of_two_becomes_a_mask
pub mod case_022 {
#[inline(never)] pub fn test_fn(value: u64) -> u64 { return value % 8; }
}
// tests/unsigned_remainder.rs::high_bit_division_remainder_and_multiplication_are_reduced
pub mod case_023 {
#[inline(never)] pub fn test_fn(value: u64) -> u64 { return value / 0x8000_0000_0000_0000u64; }
}
// tests/unsigned_remainder.rs::high_bit_division_remainder_and_multiplication_are_reduced
pub mod case_024 {
#[inline(never)] pub fn test_fn(value: u64) -> u64 { return value % 0x8000_0000_0000_0000u64; }
}
// tests/unsigned_remainder.rs::high_bit_division_remainder_and_multiplication_are_reduced
pub mod case_025 {
#[inline(never)] pub fn test_fn(value: u64) -> u64 { return value * 0x8000_0000_0000_0000u64; }
}
// tests/immediate_arithmetic.rs::arithmetic_uses_immediate_operands_without_constant_moves
pub mod case_026 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return value + 5; }
}
// tests/immediate_arithmetic.rs::bitwise_multiply_and_compare_support_immediates
pub mod case_027 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { let result = (value & 7) * 3; return result ^ 15; }
}
// tests/immediate_arithmetic.rs::large_constants_still_use_register_materialization
pub mod case_028 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return value + 2147483648; }
}
// tests/signed_division_strength.rs::dynamic_divisors_keep_all_signed_division_traps
pub mod case_029 {
#[inline(never)] pub fn test_fn(value: i64, divisor: i64) -> i64 { return value / divisor; }
}
// tests/signed_division_strength.rs::two_to_63_bit_pattern_remains_signed_division_by_a_negative
pub mod case_030 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return value / -9223372036854775808; }
}
// tests/signed_division_strength.rs::dynamic_divisors_keep_all_signed_division_traps
pub mod case_031 {
#[inline(never)] pub fn test_fn(value: i64, divisor: i64) -> i64 { return value % divisor; }
}
// tests/unsigned_remainder.rs::shared_divisor_constants_keep_their_original_values
pub mod case_032 {
#[inline(never)] pub fn test_fn(left: u64, right: u64) -> u64 { let quotient = left / 8; let remainder = right % 8; return quotient + remainder; }
}
// tests/assignment_syntax.rs::every_compound_assignment_uses_the_previous_value
pub mod case_033 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { let mut result = value; result += 5; result -= 3; result *= 4; result /= 2; result %= 7; return result; }
}
// tests/assignment_syntax.rs::mutable_parameters_can_be_assigned_and_loop_carried
pub mod case_034 {
#[inline(never)] pub fn test_fn(mut value: i64) -> i64 { value += 1; let mut i = 0; while i < 3 { value *= 2; i += 1; } return value; }
}
// tests/casts.rs::signed_and_unsigned_casts_preserve_bit_patterns
pub mod case_035 {
#[inline(never)] pub fn test_fn(value: i64) -> u64 { return value as u64; }
}
// tests/casts.rs::signed_and_unsigned_casts_preserve_bit_patterns
pub mod case_036 {
#[inline(never)] pub fn test_fn(value: u64) -> i64 { return value as i64; }
}
// tests/casts.rs::bool_casts_produce_zero_or_one
pub mod case_037 {
#[inline(never)] pub fn test_fn(flag: bool) -> i64 { return flag as i64; }
}
// tests/casts.rs::bool_casts_produce_zero_or_one
pub mod case_038 {
#[inline(never)] pub fn test_fn(flag: bool) -> usize { return flag as usize; }
}
// tests/casts.rs::casts_bind_before_arithmetic_and_can_chain
pub mod case_039 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { let bits = value as u64 as i64; let flag = bits < 0; return flag as i64 + 41; }
}
// tests/u64_ops.rs::u64_literals_and_inference_cover_the_full_bit_range
pub mod case_040 {
#[inline(never)] pub fn test_fn() -> u64 { return 18446744073709551615u64; }
}
// tests/u64_ops.rs::u64_literals_and_inference_cover_the_full_bit_range
pub mod case_041 {
#[inline(never)] pub fn test_fn() -> u64 { let value: u64 = 42; return value; }
}
// tests/u64_ops.rs::u64_literals_and_inference_cover_the_full_bit_range
pub mod case_042 {
#[inline(never)] pub fn test_fn(flag: bool) -> u64 { return if flag { 1 } else { 2u64 }; }
}
// tests/u64_ops.rs::unsigned_division_remainder_and_shifts_match_the_interpreter
pub mod case_043 {
#[inline(never)] pub fn test_fn(a: u64, b: u64) -> u64 { return a / b; }
}
// tests/u64_ops.rs::unsigned_division_remainder_and_shifts_match_the_interpreter
pub mod case_044 {
#[inline(never)] pub fn test_fn(a: u64, b: u64) -> u64 { return a % b; }
}
// tests/u64_ops.rs::unsigned_division_remainder_and_shifts_match_the_interpreter
pub mod case_045 {
#[inline(never)] pub fn test_fn(a: u64, b: u64) -> u64 { return a >> b; }
}
// tests/u64_ops.rs::unsigned_division_has_no_signed_overflow_case
pub mod case_046 {
#[inline(never)] pub fn test_fn(a: u64, b: u64) -> u64 { return a / b; }
}
// tests/u64_ops.rs::u64_arithmetic_compound_assignments_and_loops_preserve_bits
pub mod case_047 {
#[inline(never)] pub fn test_fn() -> u64 { let mut value = 0u64; value -= 1; value += 1; return value; }
}
// tests/u64_ops.rs::u64_arithmetic_compound_assignments_and_loops_preserve_bits
pub mod case_048 {
#[inline(never)] pub fn test_fn() -> u64 { let mut value = 100u64; value /= 3; value %= 30; return value; }
}
// tests/u64_ops.rs::u64_arithmetic_compound_assignments_and_loops_preserve_bits
pub mod case_049 {
#[inline(never)] pub fn test_fn() -> u64 { let mut value = 32u64; while value > 1 { value = value >> 1; } return value + 41; }
}
// tests/div_differential.rs::quotient_and_remainder_match_for_values_and_boundaries
pub mod case_050 {
#[inline(never)] pub fn test_fn(left: i64, right: i64) -> i64 { return left / right; }
}
// tests/div_differential.rs::division_by_zero_and_overflow_remain_traps
pub mod case_051 {
#[inline(never)] pub fn test_fn(left: i64, right: i64) -> i64 { return left / right; }
}
// tests/div_differential.rs::quotient_and_remainder_match_for_values_and_boundaries
pub mod case_052 {
#[inline(never)] pub fn test_fn(left: i64, right: i64) -> i64 { return left % right; }
}
// tests/div_differential.rs::division_by_zero_and_overflow_remain_traps
pub mod case_053 {
#[inline(never)] pub fn test_fn(left: i64, right: i64) -> i64 { return left % right; }
}
// tests/assignment_syntax.rs::immutable_compound_assignment_and_compound_division_errors_are_preserved
pub mod case_054 {
#[inline(never)] pub fn test_fn() -> i64 { let mut value = 1; value /= 0; return value; }
}
// tests/immediate_shifts.rs::immediate_count_is_masked_to_six_bits
pub mod case_055 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return value.wrapping_shl((-1i64) as u32); }
}
// tests/immediate_shifts.rs::immediate_count_is_masked_to_six_bits
pub mod case_056 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return value.wrapping_shl(65); }
}
// tests/integer_not.rs::not_precedence_double_inversion_and_loop_phis_are_preserved
pub mod case_057 {
#[inline(never)] pub fn test_fn(value: isize) -> isize { return !!value & 0xff; }
}
// tests/integer_not.rs::not_precedence_double_inversion_and_loop_phis_are_preserved
pub mod case_058 {
#[inline(never)] pub fn test_fn() -> i64 { let mut value = 0; let mut count = 0; while count < 3 { value = !value; count += 1; } return if value == -1 { 42 } else { 0 }; }
}
// tests/integer_not.rs::integer_not_emits_native_code_and_rejects_unit_or_signed_overflow
pub mod case_059 {
#[inline(never)] pub fn test_fn() -> i64 { return !(-43); }
}
// tests/integer_syntax.rs::combined_assignments_work_in_unsigned_loop_phi_values
pub mod case_060 {
#[inline(never)] pub fn test_fn() -> u64 { let mut value = 0x8000_0000_0000_0000u64; value >>= 63; value <<= 6; value |= 2; value ^= 0x68; value += 0; return value; }
}
// tests/integer_syntax.rs::combined_assignments_work_in_unsigned_loop_phi_values
pub mod case_061 {
#[inline(never)] pub fn test_fn() -> i64 { let mut value = 0xff; let mut count = 0; while count < 2 { value &= 0x7f; value ^= 0x20; value <<= 1; value >>= 1; value |= 1; count += 1; } return value; }
}
// tests/integer_syntax.rs::isize_is_the_signed_word_type_on_x86_64
pub mod case_062 {
fn identity(value: isize) -> isize { return value; } #[inline(never)] pub fn test_fn() -> isize { return identity(0x2aisize); }
}
// tests/integer_syntax.rs::isize_is_the_signed_word_type_on_x86_64
pub mod case_063 {
#[inline(never)] pub fn test_fn() -> isize { return 0x2aisize; }
}
// tests/integer_syntax.rs::isize_is_the_signed_word_type_on_x86_64
pub mod case_064 {
#[inline(never)] pub fn test_fn() -> isize { return -0x8000_0000_0000_0000isize; }
}
// tests/div_differential.rs::quotient_and_remainder_match_with_spills
pub mod case_065 {
#[inline(never)] pub fn test_fn(p0: i64, p1: i64, p2: i64, p3: i64, p4: i64, p5: i64, p6: i64, p7: i64, p8: i64, p9: i64, p10: i64, p11: i64, p12: i64, p13: i64, p14: i64, p15: i64, p16: i64, p17: i64, p18: i64, p19: i64, p20: i64, p21: i64, p22: i64, p23: i64) -> i64 { return ((((((p0 / p1) + (p2 * p3)) + ((p4 + p5) * (p6 - p7))) + ((p8 + p9) * (p10 - p11))) + ((p12 + p13) * (p14 - p15))) + ((p16 + p17) * (p18 - p19))) + ((p20 + p21) * (p22 - p23)); }
}
// tests/div_differential.rs::quotient_and_remainder_match_with_spills
pub mod case_066 {
#[inline(never)] pub fn test_fn(p0: i64, p1: i64, p2: i64, p3: i64, p4: i64, p5: i64, p6: i64, p7: i64, p8: i64, p9: i64, p10: i64, p11: i64, p12: i64, p13: i64, p14: i64, p15: i64, p16: i64, p17: i64, p18: i64, p19: i64, p20: i64, p21: i64, p22: i64, p23: i64) -> i64 { return ((((((p0 % p1) + (p2 * p3)) + ((p4 + p5) * (p6 - p7))) + ((p8 + p9) * (p10 - p11))) + ((p12 + p13) * (p14 - p15))) + ((p16 + p17) * (p18 - p19))) + ((p20 + p21) * (p22 - p23)); }
}
// tests/machine_differential.rs::constants_and_i64_wrapping_boundaries_match
pub mod case_067 {
#[inline(never)] pub fn test_fn() -> i64 { return 0; }
}
// tests/machine_differential.rs::constants_and_i64_wrapping_boundaries_match
pub mod case_068 {
#[inline(never)] pub fn test_fn() -> i64 { let value = 40 + 2; return value; }
}
// tests/machine_differential.rs::constants_and_i64_wrapping_boundaries_match
pub mod case_069 {
#[inline(never)] pub fn test_fn() -> i64 { return 7 - 3 * 2; }
}
// tests/machine_differential.rs::constants_and_i64_wrapping_boundaries_match
pub mod case_070 {
#[inline(never)] pub fn test_fn() -> i64 { return 9223372036854775807 + 1; }
}
// tests/machine_differential.rs::constants_and_i64_wrapping_boundaries_match
pub mod case_071 {
#[inline(never)] pub fn test_fn() -> i64 { return -9223372036854775808 - 1; }
}
// tests/machine_differential.rs::constants_and_i64_wrapping_boundaries_match
pub mod case_072 {
#[inline(never)] pub fn test_fn() -> i64 { return -9223372036854775808 * -1; }
}
// tests/machine_differential.rs::constants_and_i64_wrapping_boundaries_match
pub mod case_073 {
#[inline(never)] pub fn test_fn() -> i64 { return 9223372036854775807 * 9223372036854775807; }
}
// tests/machine_differential.rs::every_supported_arithmetic_operator_matches_at_boundaries
pub mod case_074 {
#[inline(never)] pub fn test_fn(left: i64, right: i64) -> i64 { return left + right; }
}
// tests/machine_differential.rs::every_supported_arithmetic_operator_matches_at_boundaries
pub mod case_075 {
#[inline(never)] pub fn test_fn(left: i64, right: i64) -> i64 { return left - right; }
}
// tests/machine_differential.rs::every_supported_arithmetic_operator_matches_at_boundaries
pub mod case_076 {
#[inline(never)] pub fn test_fn(left: i64, right: i64) -> i64 { return left * right; }
}
// tests/machine_differential.rs::register_and_stack_parameters_match
pub mod case_077 {
#[inline(never)] pub fn test_fn(a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64, h: i64) -> i64 { let left = (a + b) * (c - d); let right = (e * f) + (g - h); return left - right; }
}
// tests/machine_differential.rs::high_register_pressure_and_spills_match
pub mod case_078 {
#[inline(never)] pub fn test_fn(p0: i64, p1: i64, p2: i64, p3: i64, p4: i64, p5: i64, p6: i64, p7: i64, p8: i64, p9: i64, p10: i64, p11: i64, p12: i64, p13: i64, p14: i64, p15: i64, p16: i64, p17: i64, p18: i64, p19: i64, p20: i64, p21: i64, p22: i64, p23: i64) -> i64 { return ((((((p0 + p1) * (p2 - p3)) + ((p4 | p5) + (p6 * p7))) + ((p8 - p9) * (p10 + p11))) + ((p12 + p13) * (p14 - p15))) + ((p16 | p17) + (p18 * p19))) + ((p20 - p21) * (p22 + p23)); }
}
// tests/integer_syntax.rs::bitwise_and_shift_compound_assignments_match_the_interpreter
pub mod case_079 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> i64 { let mut value = a; value &= b; return value; }
}
// tests/integer_syntax.rs::bitwise_and_shift_compound_assignments_match_the_interpreter
pub mod case_080 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> i64 { let mut value = a; value |= b; return value; }
}
// tests/integer_syntax.rs::bitwise_and_shift_compound_assignments_match_the_interpreter
pub mod case_081 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> i64 { let mut value = a; value ^= b; return value; }
}
// tests/integer_syntax.rs::bitwise_and_shift_compound_assignments_match_the_interpreter
pub mod case_082 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> i64 { let mut value = a; value <<= b; return value; }
}
// tests/integer_syntax.rs::bitwise_and_shift_compound_assignments_match_the_interpreter
pub mod case_083 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> i64 { let mut value = a; value >>= b; return value; }
}
// tests/signed_division_strength.rs::positive_powers_avoid_idiv_and_round_toward_zero
pub mod case_084 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return value / 2; }
}
// tests/signed_division_strength.rs::positive_powers_avoid_idiv_and_round_toward_zero
pub mod case_085 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return value / 4; }
}
// tests/signed_division_strength.rs::positive_powers_avoid_idiv_and_round_toward_zero
pub mod case_086 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return value / 8; }
}
// tests/signed_division_strength.rs::positive_powers_avoid_idiv_and_round_toward_zero
pub mod case_087 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return value / 2147483648; }
}
// tests/signed_division_strength.rs::positive_powers_avoid_idiv_and_round_toward_zero
pub mod case_088 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return value / 4294967296; }
}
// tests/signed_division_strength.rs::positive_powers_avoid_idiv_and_round_toward_zero
pub mod case_089 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return value / 4611686018427387904; }
}
// tests/signed_division_strength.rs::native_division_by_one_is_a_copy
pub mod case_090 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return value / 1; }
}
// tests/integer_ops.rs::son_bitwise_not_and_direct_objects_are_supported
pub mod case_091 {
#[inline(never)] pub fn test_fn() -> i64 { return !0i64; }
}
// tests/native_pipeline.rs::optimized_graph_is_placed_and_compiled_without_llvm
pub mod case_092 {
#[inline(never)] pub fn test_fn() -> i64 { return 40 + 2; }
}
// tests/native_pipeline.rs::optimization_preserves_interpreted_results
pub mod case_093 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { let left = value + 0; return left * 1; }
}
// tests/signed_division_strength.rs::direct_object_emission_accepts_reduced_division
pub mod case_094 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return value / 4; }
}
// tests/signed_division_strength.rs::reduced_division_handles_spilled_inputs_and_results
pub mod case_095 {
#[inline(never)] pub fn test_fn(p0: i64, p1: i64, p2: i64, p3: i64, p4: i64, p5: i64, p6: i64, p7: i64, p8: i64, p9: i64, p10: i64, p11: i64, p12: i64, p13: i64, p14: i64, p15: i64, p16: i64, p17: i64, p18: i64, p19: i64) -> i64 { let quotient = p0 / 4; let sum = p1 + p2 + p3 + p4 + p5 + p6 + p7 + p8 + p9 + p10 + p11 + p12 + p13 + p14 + p15 + p16 + p17 + p18 + p19; return sum + quotient; }
}
// tests/signed_division_strength.rs::reduced_division_handles_spilled_inputs_and_results
pub mod case_096 {
#[inline(never)] pub fn test_fn(p0: i64, p1: i64, p2: i64, p3: i64, p4: i64, p5: i64, p6: i64, p7: i64, p8: i64, p9: i64, p10: i64, p11: i64, p12: i64, p13: i64, p14: i64, p15: i64, p16: i64, p17: i64, p18: i64, p19: i64) -> i64 { let sum = p1 + p2 + p3 + p4 + p5 + p6 + p7 + p8 + p9 + p10 + p11 + p12 + p13 + p14 + p15 + p16 + p17 + p18 + p19; let quotient = p0 / 4; return sum + quotient; }
}
// tests/casts.rs::casts_bind_before_arithmetic_and_can_chain
pub mod case_097 {
#[inline(never)] pub fn test_fn(value: i64) -> u64 { return value as u64 / 2; }
}
