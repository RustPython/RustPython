"""Tests for auto_mark.py - test result parsing and auto-marking."""

import subprocess
import unittest

from update_lib.auto_mark import (
    Test,
    TestResult,
    apply_test_changes,
    collect_test_changes,
    extract_test_methods,
    parse_results,
    path_to_test_parts,
    remove_expected_failures,
)
from update_lib.patch_spec import COMMENT


class TestParseResults(unittest.TestCase):
    """Tests for parse_results function."""

    def _make_result(self, stdout: str) -> subprocess.CompletedProcess:
        """Create a mock CompletedProcess."""
        return subprocess.CompletedProcess(
            args=["test"],
            returncode=0,
            stdout=stdout,
            stderr="",
        )

    def test_parse_failing_test(self):
        """Test parsing a failing test."""
        stdout = """
Run 1 tests sequentially
test_foo (test.test_example.TestClass) ... FAIL
-----------
"""
        result = parse_results(self._make_result(stdout))
        self.assertEqual(len(result.tests), 1)
        self.assertEqual(result.tests[0].name, "test_foo")
        self.assertEqual(result.tests[0].path, "test.test_example.TestClass")
        self.assertEqual(result.tests[0].result, "fail")

    def test_parse_error_test(self):
        """Test parsing an error test."""
        stdout = """
Run 1 tests sequentially
test_bar (test.test_example.TestClass) ... ERROR
-----------
"""
        result = parse_results(self._make_result(stdout))
        self.assertEqual(len(result.tests), 1)
        self.assertEqual(result.tests[0].result, "error")

    def test_parse_ok_test_ignored(self):
        """Test that passing tests are ignored."""
        stdout = """
Run 1 tests sequentially
test_foo (test.test_example.TestClass) ... ok
-----------
"""
        result = parse_results(self._make_result(stdout))
        self.assertEqual(len(result.tests), 0)

    def test_parse_unexpected_success(self):
        """Test parsing unexpected success."""
        stdout = """
Run 1 tests sequentially
test_foo (test.test_example.TestClass) ... unexpected success
-----------
UNEXPECTED SUCCESS: test_foo (test.test_example.TestClass)
"""
        result = parse_results(self._make_result(stdout))
        self.assertEqual(len(result.unexpected_successes), 1)
        self.assertEqual(result.unexpected_successes[0].name, "test_foo")
        self.assertEqual(
            result.unexpected_successes[0].path, "test.test_example.TestClass"
        )

    def test_parse_tests_result(self):
        """Test parsing tests result line."""
        stdout = """
== Tests result: FAILURE ==
"""
        result = parse_results(self._make_result(stdout))
        self.assertEqual(result.tests_result, "FAILURE")

    def test_parse_multiple_tests(self):
        """Test parsing multiple test results."""
        stdout = """
Run 3 tests sequentially
test_one (test.test_example.TestA) ... FAIL
test_two (test.test_example.TestA) ... ok
test_three (test.test_example.TestB) ... ERROR
-----------
"""
        result = parse_results(self._make_result(stdout))
        self.assertEqual(len(result.tests), 2)  # Only FAIL and ERROR

    def test_parse_error_message(self):
        """Test parsing error message from traceback."""
        stdout = """
Run 1 tests sequentially
test_foo (test.test_example.TestClass) ... FAIL
-----------
======================================================================
FAIL: test_foo (test.test_example.TestClass)
----------------------------------------------------------------------
Traceback (most recent call last):
  File "test.py", line 10, in test_foo
    self.assertEqual(1, 2)
AssertionError: 1 != 2

======================================================================
"""
        result = parse_results(self._make_result(stdout))
        self.assertEqual(len(result.tests), 1)
        self.assertEqual(result.tests[0].error_message, "AssertionError: 1 != 2")

    def test_parse_multiple_error_messages(self):
        """Test parsing multiple error messages."""
        stdout = """
Run 2 tests sequentially
test_foo (test.test_example.TestClass) ... FAIL
test_bar (test.test_example.TestClass) ... ERROR
-----------
======================================================================
FAIL: test_foo (test.test_example.TestClass)
----------------------------------------------------------------------
Traceback (most recent call last):
  File "test.py", line 10, in test_foo
    self.assertEqual(1, 2)
AssertionError: 1 != 2

======================================================================
ERROR: test_bar (test.test_example.TestClass)
----------------------------------------------------------------------
Traceback (most recent call last):
  File "test.py", line 20, in test_bar
    raise ValueError("oops")
ValueError: oops

======================================================================
"""
        result = parse_results(self._make_result(stdout))
        self.assertEqual(len(result.tests), 2)
        # Find tests by name
        test_foo = next(t for t in result.tests if t.name == "test_foo")
        test_bar = next(t for t in result.tests if t.name == "test_bar")
        self.assertEqual(test_foo.error_message, "AssertionError: 1 != 2")
        self.assertEqual(test_bar.error_message, "ValueError: oops")


class TestPathToTestParts(unittest.TestCase):
    """Tests for path_to_test_parts function."""

    def test_simple_path(self):
        """Test extracting parts from simple path."""
        parts = path_to_test_parts("test.test_foo.TestClass.test_method")
        self.assertEqual(parts, ["TestClass", "test_method"])

    def test_nested_path(self):
        """Test extracting parts from nested path."""
        parts = path_to_test_parts("test.test_foo.test_bar.TestClass.test_method")
        self.assertEqual(parts, ["TestClass", "test_method"])


class TestCollectTestChanges(unittest.TestCase):
    """Tests for collect_test_changes function."""

    def test_collect_failing_tests(self):
        """Test collecting failing tests."""
        results = TestResult()
        results.tests = [
            Test(
                name="test_foo",
                path="test.test_example.TestClass.test_foo",
                result="fail",
            ),
            Test(
                name="test_bar",
                path="test.test_example.TestClass.test_bar",
                result="error",
            ),
        ]

        failing, successes, error_messages = collect_test_changes(results)

        self.assertEqual(len(failing), 2)
        self.assertIn(("TestClass", "test_foo"), failing)
        self.assertIn(("TestClass", "test_bar"), failing)
        self.assertEqual(len(successes), 0)

    def test_collect_unexpected_successes(self):
        """Test collecting unexpected successes."""
        results = TestResult()
        results.unexpected_successes = [
            Test(
                name="test_foo",
                path="test.test_example.TestClass.test_foo",
                result="unexpected_success",
            ),
        ]

        failing, successes, error_messages = collect_test_changes(results)

        self.assertEqual(len(failing), 0)
        self.assertEqual(len(successes), 1)
        self.assertIn(("TestClass", "test_foo"), successes)

    def test_collect_with_module_prefix(self):
        """Test collecting with module prefix filter."""
        results = TestResult()
        results.tests = [
            Test(name="test_foo", path="test_a.TestClass.test_foo", result="fail"),
            Test(name="test_bar", path="test_b.TestClass.test_bar", result="fail"),
        ]

        failing, _, _ = collect_test_changes(results, module_prefix="test_a.")

        self.assertEqual(len(failing), 1)
        self.assertIn(("TestClass", "test_foo"), failing)

    def test_collect_error_messages(self):
        """Test collecting error messages."""
        results = TestResult()
        results.tests = [
            Test(
                name="test_foo",
                path="test.test_example.TestClass.test_foo",
                result="fail",
                error_message="AssertionError: 1 != 2",
            ),
            Test(
                name="test_bar",
                path="test.test_example.TestClass.test_bar",
                result="error",
                error_message="",
            ),
        ]

        failing, successes, error_messages = collect_test_changes(results)

        self.assertEqual(len(error_messages), 1)
        self.assertEqual(
            error_messages[("TestClass", "test_foo")], "AssertionError: 1 != 2"
        )

    def test_collect_with_test_prefix_in_path(self):
        """Test collecting with 'test.' prefix in path (like real test output)."""
        results = TestResult()
        results.tests = [
            Test(
                name="test_foo",
                path="test.test_dataclasses.TestCase.test_foo",
                result="fail",
            ),
            Test(
                name="test_bar",
                path="test.test_other.TestOther.test_bar",
                result="fail",
            ),
        ]

        # Filter with prefix that matches real test module path format
        failing, _, _ = collect_test_changes(
            results, module_prefix="test.test_dataclasses."
        )

        self.assertEqual(len(failing), 1)
        self.assertIn(("TestCase", "test_foo"), failing)

    def test_collect_init_module_matching(self):
        """Test that __init__.py tests match without __init__ in path.

        When test results come from a package's __init__.py, the path is like:
        'test.test_dataclasses.TestCase.test_foo' (no __init__)

        But module_prefix from test_name_from_path would be:
        'test_dataclasses.__init__'

        So we need to strip '.__init__' and add 'test.' prefix.
        """
        results = TestResult()
        results.tests = [
            Test(
                name="test_field_repr",
                path="test.test_dataclasses.TestCase.test_field_repr",
                result="fail",
            ),
        ]

        # Simulate the corrected prefix (after stripping .__init__ and adding test.)
        module_prefix = "test_dataclasses.__init__"
        if module_prefix.endswith(".__init__"):
            module_prefix = module_prefix[:-9]
        module_prefix = "test." + module_prefix + "."

        failing, _, _ = collect_test_changes(results, module_prefix=module_prefix)

        self.assertEqual(len(failing), 1)
        self.assertIn(("TestCase", "test_field_repr"), failing)


class TestExtractTestMethods(unittest.TestCase):
    """Tests for extract_test_methods function."""

    def test_extract_simple(self):
        """Test extracting test methods from simple class."""
        code = """
class TestFoo(unittest.TestCase):
    def test_one(self):
        pass

    def test_two(self):
        pass
"""
        methods = extract_test_methods(code)
        self.assertEqual(len(methods), 2)
        self.assertIn(("TestFoo", "test_one"), methods)
        self.assertIn(("TestFoo", "test_two"), methods)

    def test_extract_multiple_classes(self):
        """Test extracting from multiple classes."""
        code = """
class TestA(unittest.TestCase):
    def test_a(self):
        pass

class TestB(unittest.TestCase):
    def test_b(self):
        pass
"""
        methods = extract_test_methods(code)
        self.assertEqual(len(methods), 2)
        self.assertIn(("TestA", "test_a"), methods)
        self.assertIn(("TestB", "test_b"), methods)

    def test_extract_syntax_error_returns_empty(self):
        """Test that syntax error returns empty set."""
        code = "this is not valid python {"
        methods = extract_test_methods(code)
        self.assertEqual(methods, set())


class TestRemoveExpectedFailures(unittest.TestCase):
    """Tests for remove_expected_failures function."""

    def test_remove_simple(self):
        """Test removing simple expectedFailure decorator."""
        code = f"""import unittest

class TestFoo(unittest.TestCase):
    # {COMMENT}
    @unittest.expectedFailure
    def test_one(self):
        pass
"""
        result = remove_expected_failures(code, {("TestFoo", "test_one")})
        self.assertNotIn("@unittest.expectedFailure", result)
        self.assertIn("def test_one(self):", result)

    def test_remove_with_inline_comment(self):
        """Test removing expectedFailure with inline comment."""
        code = f"""import unittest

class TestFoo(unittest.TestCase):
    @unittest.expectedFailure  # {COMMENT}
    def test_one(self):
        pass
"""
        result = remove_expected_failures(code, {("TestFoo", "test_one")})
        self.assertNotIn("@unittest.expectedFailure", result)

    def test_remove_super_call_method(self):
        """Test removing method that just calls super()."""
        code = f"""import unittest

class TestFoo(unittest.TestCase):
    # {COMMENT}
    @unittest.expectedFailure
    def test_one(self):
        return super().test_one()
"""
        result = remove_expected_failures(code, {("TestFoo", "test_one")})
        self.assertNotIn("def test_one", result)

    def test_no_removal_without_comment(self):
        """Test that decorators without COMMENT are not removed."""
        code = """import unittest

class TestFoo(unittest.TestCase):
    @unittest.expectedFailure
    def test_one(self):
        pass
"""
        result = remove_expected_failures(code, {("TestFoo", "test_one")})
        # Should still have the decorator
        self.assertIn("@unittest.expectedFailure", result)


class TestApplyTestChanges(unittest.TestCase):
    """Tests for apply_test_changes function."""

    def test_apply_failing_tests(self):
        """Test applying expectedFailure to failing tests."""
        code = """import unittest

class TestFoo(unittest.TestCase):
    def test_one(self):
        pass
"""
        failing = {("TestFoo", "test_one")}
        result = apply_test_changes(code, failing, set())

        self.assertIn("@unittest.expectedFailure", result)
        self.assertIn(COMMENT, result)

    def test_apply_removes_unexpected_success(self):
        """Test removing expectedFailure from unexpected success."""
        code = f"""import unittest

class TestFoo(unittest.TestCase):
    # {COMMENT}
    @unittest.expectedFailure
    def test_one(self):
        pass
"""
        successes = {("TestFoo", "test_one")}
        result = apply_test_changes(code, set(), successes)

        self.assertNotIn("@unittest.expectedFailure", result)
        self.assertIn("def test_one(self):", result)

    def test_apply_both_changes(self):
        """Test applying both failing tests and removing unexpected successes."""
        code = f"""import unittest

class TestFoo(unittest.TestCase):
    def test_one(self):
        pass

    # {COMMENT}
    @unittest.expectedFailure
    def test_two(self):
        pass
"""
        failing = {("TestFoo", "test_one")}
        successes = {("TestFoo", "test_two")}
        result = apply_test_changes(code, failing, successes)

        # test_one should now have expectedFailure
        self.assertIn("def test_one(self):", result)
        # Only one expectedFailure decorator should remain (on test_one)
        self.assertEqual(result.count("@unittest.expectedFailure"), 1)

    def test_apply_with_error_message(self):
        """Test applying expectedFailure with error message."""
        code = """import unittest

class TestFoo(unittest.TestCase):
    def test_one(self):
        pass
"""
        failing = {("TestFoo", "test_one")}
        error_messages = {("TestFoo", "test_one"): "AssertionError: 1 != 2"}
        result = apply_test_changes(code, failing, set(), error_messages)

        self.assertIn("@unittest.expectedFailure", result)
        self.assertIn("AssertionError: 1 != 2", result)
        self.assertIn(COMMENT, result)


class TestSmartAutoMarkFiltering(unittest.TestCase):
    """Tests for smart auto-mark filtering logic (regression exclusion).

    The smart auto-mark feature:
    - Marks NEW test failures (tests that didn't exist before)
    - Does NOT mark regressions (existing tests that now fail)
    """

    def _filter_failures(
        self,
        all_failing_tests: set[tuple[str, str]],
        original_methods: set[tuple[str, str]],
        current_methods: set[tuple[str, str]],
    ) -> tuple[set[tuple[str, str]], set[tuple[str, str]]]:
        """Simulate the filtering logic from auto_mark_file().

        Returns:
            (failing_tests_to_mark, regressions)
        """
        new_methods = current_methods - original_methods
        failing_tests = {t for t in all_failing_tests if t in new_methods}
        regressions = all_failing_tests - failing_tests
        return failing_tests, regressions

    def test_new_tests_get_marked(self):
        """Test that new failing tests are marked."""
        original_methods = {("TestFoo", "test_existing")}
        current_methods = {("TestFoo", "test_existing"), ("TestFoo", "test_new")}
        all_failing = {("TestFoo", "test_new")}

        to_mark, regressions = self._filter_failures(
            all_failing, original_methods, current_methods
        )

        self.assertEqual(to_mark, {("TestFoo", "test_new")})
        self.assertEqual(regressions, set())

    def test_regressions_not_marked(self):
        """Test that existing failing tests (regressions) are NOT marked."""
        original_methods = {("TestFoo", "test_existing")}
        current_methods = {("TestFoo", "test_existing")}
        all_failing = {("TestFoo", "test_existing")}

        to_mark, regressions = self._filter_failures(
            all_failing, original_methods, current_methods
        )

        self.assertEqual(to_mark, set())
        self.assertEqual(regressions, {("TestFoo", "test_existing")})

    def test_mixed_new_and_regression(self):
        """Test with both new failures and regressions."""
        original_methods = {("TestFoo", "test_old1"), ("TestFoo", "test_old2")}
        current_methods = {
            ("TestFoo", "test_old1"),
            ("TestFoo", "test_old2"),
            ("TestFoo", "test_new1"),
            ("TestFoo", "test_new2"),
        }
        # test_old1 regressed, test_new1 is a new failure
        all_failing = {("TestFoo", "test_old1"), ("TestFoo", "test_new1")}

        to_mark, regressions = self._filter_failures(
            all_failing, original_methods, current_methods
        )

        self.assertEqual(to_mark, {("TestFoo", "test_new1")})
        self.assertEqual(regressions, {("TestFoo", "test_old1")})

    def test_multiple_classes(self):
        """Test filtering across multiple classes."""
        original_methods = {("TestA", "test_a"), ("TestB", "test_b")}
        current_methods = {
            ("TestA", "test_a"),
            ("TestA", "test_new_a"),
            ("TestB", "test_b"),
            ("TestC", "test_c"),  # entirely new class
        }
        all_failing = {
            ("TestA", "test_a"),  # regression
            ("TestA", "test_new_a"),  # new
            ("TestC", "test_c"),  # new (new class)
        }

        to_mark, regressions = self._filter_failures(
            all_failing, original_methods, current_methods
        )

        self.assertEqual(to_mark, {("TestA", "test_new_a"), ("TestC", "test_c")})
        self.assertEqual(regressions, {("TestA", "test_a")})

    def test_all_new_tests(self):
        """Test when all failing tests are new (no regressions)."""
        original_methods = set()  # file was new
        current_methods = {("TestFoo", "test_one"), ("TestFoo", "test_two")}
        all_failing = {("TestFoo", "test_one"), ("TestFoo", "test_two")}

        to_mark, regressions = self._filter_failures(
            all_failing, original_methods, current_methods
        )

        self.assertEqual(to_mark, all_failing)
        self.assertEqual(regressions, set())

    def test_all_regressions(self):
        """Test when all failing tests are regressions (no new tests)."""
        original_methods = {("TestFoo", "test_one"), ("TestFoo", "test_two")}
        current_methods = original_methods.copy()
        all_failing = {("TestFoo", "test_one")}

        to_mark, regressions = self._filter_failures(
            all_failing, original_methods, current_methods
        )

        self.assertEqual(to_mark, set())
        self.assertEqual(regressions, {("TestFoo", "test_one")})


if __name__ == "__main__":
    unittest.main()
