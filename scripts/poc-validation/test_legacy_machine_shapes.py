"""The audit must interpret equivalent LLVM operand spellings identically."""
import importlib.util
from pathlib import Path
import unittest


spec = importlib.util.spec_from_file_location(
    'machine_shapes', Path(__file__).with_name('legacy-machine-shapes.py'))
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class OperandSpelling(unittest.TestCase):
    def check_both(self, test, decimal, hexadecimal, case='example'):
        results = []
        for assembly in (decimal, hexadecimal):
            instructions = [dict(op=op, args=args, bytes=[]) for op, args in assembly]
            original = [dict(i) for i in instructions]
            results.append(audit.obligations(test, {'id': case}, instructions))
            self.assertEqual(instructions, original, 'raw evidence must stay unchanged')
        self.assertEqual(results[0], results[1])
        self.assertTrue(all(row['passed'] for row in results[0]), results[0])

    def test_arithmetic_and_comparison(self):
        self.check_both('arithmetic_uses_immediate_operands_without_constant_moves',
                        [('add', 'rax, 5')], [('add', 'rax, 0x5')])
        self.check_both('subtraction_keeps_the_constant_on_the_left',
                        [('mov', 'rsi, 42')], [('mov', 'rsi, 0x2a')])
        self.check_both('bitwise_multiply_and_compare_support_immediates',
                        [('and', 'rax, 7'), ('imul', 'rax, 3'), ('xor', 'rax, 15')],
                        [('and', 'rax, 0x7'), ('imul', 'rax, 0x3'), ('xor', 'rax, 0xf')])
        self.check_both('bitwise_multiply_and_compare_support_immediates',
                        [('cmp', 'rax, 42')], [('cmp', 'rax, 0x2a')], 'x__compare')

    def test_spills_and_signed_shift_sequence(self):
        self.check_both('shifts_survive_spills_and_loop_phi_values',
                        [('mov', 'rax, qword ptr [rbp - 112]')],
                        [('mov', 'rax, qword ptr [rbp - 0x70]')])
        self.check_both('reduced_division_handles_spilled_inputs_and_results',
                        [('sar', 'rax, 63'), ('shr', 'rax, 62'), ('add', 'rax, rdx'),
                         ('sar', 'rax, 2'), ('mov', 'qword ptr [rbp - 112], rax')],
                        [('sar', 'rax, 0x3f'), ('shr', 'rax, 0x3e'), ('add', 'rax, rdx'),
                         ('sar', 'rax, 0x2'), ('mov', 'qword ptr [rbp - 0x70], rax')],
                        'x__early')

    def test_wrong_immediates_and_callee_save_slots_still_fail(self):
        cases = [
            ('arithmetic_uses_immediate_operands_without_constant_moves', 'add', 'rax, 0x6'),
            ('subtraction_keeps_the_constant_on_the_left', 'mov', 'rax, 0x2b'),
            ('loop_survives_register_pressure', 'mov', 'qword ptr [rbp - 0x70], rbx'),
            ('loop_survives_register_pressure', 'mov', 'rax, qword ptr [rsp - 0x70]'),
        ]
        for test, op, args in cases:
            with self.subTest(test=test, args=args):
                checks = audit.obligations(test, {'id': 'x'}, [dict(op=op, args=args, bytes=[])])
                self.assertFalse(all(row['passed'] for row in checks))


if __name__ == '__main__':
    unittest.main()
