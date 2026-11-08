import unittest
from unittest.mock import patch

import test_snippets


class SnippetSelectionTests(unittest.TestCase):
    def make_test(self, method, name):
        class Tests:
            pass

        test_snippets.create_test_function(Tests, f"snippets/{name}.py", method, None)
        return getattr(Tests, f"test_{method}_{name}")

    def test_exclusion_matches_the_whole_snippet_name(self):
        with patch.object(test_snippets, "RUSTPYTHON_SNIPPET_EXCLUDE", {"builtin_str"}):
            excluded = self.make_test("rustpython", "builtin_str")
            sibling = self.make_test("rustpython", "builtin_str_add")

        self.assertTrue(excluded.__unittest_skip__)
        self.assertFalse(getattr(sibling, "__unittest_skip__", False))

    def test_exclusion_does_not_change_cpython_reference_tests(self):
        with patch.object(test_snippets, "RUSTPYTHON_SNIPPET_EXCLUDE", {"builtin_str"}):
            reference = self.make_test("cpython", "builtin_str")

        self.assertFalse(getattr(reference, "__unittest_skip__", False))

    def test_no_exclusions_keeps_the_native_default(self):
        with patch.object(test_snippets, "RUSTPYTHON_SNIPPET_EXCLUDE", set()):
            test = self.make_test("rustpython", "builtin_str")

        self.assertFalse(getattr(test, "__unittest_skip__", False))
