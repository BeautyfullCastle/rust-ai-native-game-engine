#!/usr/bin/env python3
"""Synthetic positive and adversarial fixtures for the independent GPU oracle."""
import unittest
import static_model_assemble_gpu_check as checker


class Fixtures(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.checked, cls.counts, cls.background = checker.oracle()
        cls.good = bytearray(checker.SIZE * checker.SIZE * 4)
        for offset, color in cls.checked:
            cls.good[offset:offset + 4] = bytes(color)
        cls.interiors = [(o, c) for o, c in cls.checked if c != (0, 0, 0, 255)]

    def test_positive_scalar_frame(self):
        checker._assert_frame(self.good, self.checked, 'positive')

    def test_fixed_coverage_and_distinct_normals(self):
        self.assertEqual(self.counts, [363, 347, 390, 391, 390, 363])
        self.assertEqual(self.background, 141514)
        self.assertEqual(len(self.checked), 143758)
        colors = [checker._geometry(p, n)[1] for p in range(3) for n in range(2)]
        self.assertEqual(len(set(colors)), 6)

    def test_each_missing_part_rejected(self):
        for part in range(3):
            bad = self.good.copy()
            colors = {checker._geometry(part, n)[1] for n in range(2)}
            for offset, color in self.interiors:
                if color in colors:
                    bad[offset:offset + 4] = bytes((0, 0, 0, 255))
            with self.assertRaises(AssertionError):
                checker._assert_frame(bad, self.checked, 'missing part')

    def test_material_or_image_collision_rejected(self):
        bad = self.good.copy()
        wrong = checker._geometry(0, 0)[1]
        expected = checker._geometry(1, 0)[1]
        for offset, color in self.interiors:
            if color == expected:
                bad[offset:offset + 4] = bytes(wrong)
        with self.assertRaises(AssertionError):
            checker._assert_frame(bad, self.checked, 'wrong table offset')

    def test_wrong_normal_lighting_rejected(self):
        bad = self.good.copy()
        # Same source/material repeated under a different normal transform.
        wrong = checker._geometry(2, 0)[1]
        expected = checker._geometry(0, 0)[1]
        self.assertGreater(max(abs(a-b) for a,b in zip(wrong, expected)), checker.EPSILON)
        for offset, color in self.interiors:
            if color == expected:
                bad[offset:offset + 4] = bytes(wrong)
        with self.assertRaises(AssertionError):
            checker._assert_frame(bad, self.checked, 'wrong normal')

    def test_background_corruption_rejected(self):
        bad = self.good.copy()
        offset = next(o for o,c in self.checked if c == (0, 0, 0, 255))
        bad[offset] = checker.EPSILON + 1
        with self.assertRaises(AssertionError):
            checker._assert_frame(bad, self.checked, 'background')

    def test_tolerance_boundary(self):
        offset, _ = self.interiors[0]
        bad = self.good.copy()
        bad[offset] += checker.EPSILON
        checker._assert_frame(bad, self.checked, 'allowed rounding')
        bad[offset] += 1
        with self.assertRaises(AssertionError):
            checker._assert_frame(bad, self.checked, 'beyond rounding')

    def test_wrong_length_rejected(self):
        for bad in (self.good[:-4], self.good + bytes(4)):
            with self.assertRaises(AssertionError):
                checker._assert_frame(bad, self.checked, 'truncated or expanded')


if __name__ == '__main__':
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(Fixtures)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    if not result.wasSuccessful() or result.testsRun != 8 or result.skipped:
        raise SystemExit(1)
    print('STATIC_ASSEMBLE_GPU_CHECKER_FIXTURES: 8 passed; 0 failed; 0 skipped')
