"""Source-only fixtures for exact canonical doctest guard migration."""

import unittest

from update_lib.doctest_patches import (
    CANONICAL_LOAD_TESTS,
    PATCHED_LOAD_TESTS,
    apply_doctest_metadata,
)

SOURCE = "raise X, Y\n"
METADATA = {SOURCE: "known mismatch"}
OLD = (
    '''"""
>>> raise X, Y  # TODO: RUSTPYTHON; old reason # doctest: +EXPECTED_FAILURE
Traceback (most recent call last):
SyntaxError: old message
"""
'''
    + PATCHED_LOAD_TESTS
    + "\n"
)
CANONICAL = (
    '''"""
>>> raise X, Y
Traceback (most recent call last):
SyntaxError: final message
"""
'''
    + CANONICAL_LOAD_TESTS
    + "\n"
)


class ExactDoctestTests(unittest.TestCase):
    def test_preserves_final_output_and_roundtrips(self):
        result = apply_doctest_metadata(OLD, CANONICAL, METADATA)
        self.assertIn("SyntaxError: final message", result)
        self.assertNotIn("old message", result)
        self.assertEqual(result.count("doctest: +EXPECTED_FAILURE"), 1)
        self.assertIn(PATCHED_LOAD_TESTS, result)
        self.assertEqual(apply_doctest_metadata(result, CANONICAL, METADATA), result)

    def test_duplicate_input_is_rejected(self):
        duplicate = CANONICAL.replace(
            ">>> raise X, Y\n", ">>> raise X, Y\n>>> raise X, Y\n"
        )
        with self.assertRaises(ValueError):
            apply_doctest_metadata(OLD, duplicate, METADATA)

    def test_missing_canonical_or_old_case_is_rejected(self):
        with self.assertRaises(ValueError):
            apply_doctest_metadata(
                OLD, CANONICAL.replace("raise X, Y", "raise X"), METADATA
            )
        with self.assertRaises(ValueError):
            apply_doctest_metadata(
                OLD.replace("raise X, Y", "raise X"), CANONICAL, METADATA
            )

    def test_changed_loader_and_unmarked_old_case_are_rejected(self):
        with self.assertRaises(ValueError):
            apply_doctest_metadata(
                OLD, CANONICAL.replace("DocTestSuite()", "DocTestSuite(None)"), METADATA
            )
        with self.assertRaises(ValueError):
            apply_doctest_metadata(CANONICAL, CANONICAL, METADATA)
