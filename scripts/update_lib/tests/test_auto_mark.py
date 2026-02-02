"""Tests for auto_mark.py - test result parsing and auto-marking."""

import ast
import pathlib
import subprocess
import tempfile
import unittest
from unittest import mock

from update_lib.cmd_auto_mark import (
    Test,
    TestResult,
    TestRunError,
    _expand_stripped_to_children,
    _is_super_call_only,
    apply_test_changes,
    auto_mark_directory,
    auto_mark_file,
    collect_test_changes,
    extract_test_methods,
    parse_results,
    path_to_test_parts,
    remove_expected_failures,
    strip_reasonless_expected_failures,
)
from update_lib.patch_spec import COMMENT


def _make_result(stdout: str) -> subprocess.CompletedProcess:
    return subprocess.CompletedProcess(
        args=["test"], returncode=0, stdout=stdout, stderr=""
    )


# -- fixtures shared across inheritance-aware tests --

BASE_TWO_CHILDREN = """import unittest

class Base:
    def test_foo(self):
        pass

class ChildA(Base, unittest.TestCase):
    pass

class ChildB(Base, unittest.TestCase):
    pass
"""

BASE_TWO_CHILDREN_ONE_OVERRIDE = """import unittest

class Base:
    def test_foo(self):
        pass

class ChildA(Base, unittest.TestCase):
    pass

class ChildB(Base, unittest.TestCase):
    def test_foo(self):
        # own implementation
        pass
"""


class TestParseResults(unittest.TestCase):
    """Tests for parse_results function."""

    def test_parse_fail_and_error(self):
        """FAIL and ERROR are collected; ok is ignored."""
        stdout = """\
Run 3 tests sequentially
test_one (test.test_example.TestA.test_one) ... FAIL
test_two (test.test_example.TestA.test_two) ... ok
test_three (test.test_example.TestB.test_three) ... ERROR
-----------
"""
        result = parse_results(_make_result(stdout))
        self.assertEqual(len(result.tests), 2)
        by_name = {t.name: t for t in result.tests}
        self.assertEqual(by_name["test_one"].path, "test.test_example.TestA.test_one")
        self.assertEqual(by_name["test_one"].result, "fail")
        self.assertEqual(by_name["test_three"].result, "error")

    def test_parse_unexpected_success(self):
        stdout = """\
Run 1 tests sequentially
test_foo (test.test_example.TestClass.test_foo) ... unexpected success
-----------
UNEXPECTED SUCCESS: test_foo (test.test_example.TestClass.test_foo)
"""
        result = parse_results(_make_result(stdout))
        self.assertEqual(len(result.unexpected_successes), 1)
        self.assertEqual(result.unexpected_successes[0].name, "test_foo")
        self.assertEqual(
            result.unexpected_successes[0].path, "test.test_example.TestClass.test_foo"
        )

    def test_parse_tests_result(self):
        result = parse_results(_make_result("== Tests result: FAILURE ==\n"))
        self.assertEqual(result.tests_result, "FAILURE")

    def test_parse_crashed_run_no_tests_result(self):
        """Test results are still parsed when the runner crashes (no Tests result line)."""
        stdout = """\
Run 1 test sequentially in a single process
0:00:00 [1/1] test_ast
test_foo (test.test_ast.test_ast.TestA.test_foo) ... FAIL
test_bar (test.test_ast.test_ast.TestA.test_bar) ... ok
test_baz (test.test_ast.test_ast.TestB.test_baz) ... ERROR
"""
        result = parse_results(_make_result(stdout))
        self.assertEqual(result.tests_result, "")
        self.assertEqual(len(result.tests), 2)
        names = {t.name for t in result.tests}
        self.assertIn("test_foo", names)
        self.assertIn("test_baz", names)

    def test_parse_crashed_run_has_unexpected_success(self):
        """Unexpected successes are parsed even without Tests result line."""
        stdout = """\
Run 1 test sequentially in a single process
0:00:00 [1/1] test_ast
test_foo (test.test_ast.test_ast.TestA.test_foo) ... unexpected success
UNEXPECTED SUCCESS: test_foo (test.test_ast.test_ast.TestA.test_foo)
"""
        result = parse_results(_make_result(stdout))
        self.assertEqual(result.tests_result, "")
        self.assertEqual(len(result.unexpected_successes), 1)

    def test_parse_error_messages(self):
        """Single and multiple error messages are parsed from tracebacks."""
        stdout = """\
Run 2 tests sequentially
test_foo (test.test_example.TestClass.test_foo) ... FAIL
test_bar (test.test_example.TestClass.test_bar) ... ERROR
-----------
======================================================================
FAIL: test_foo (test.test_example.TestClass.test_foo)
----------------------------------------------------------------------
Traceback (most recent call last):
  File "test.py", line 10, in test_foo
    self.assertEqual(1, 2)
AssertionError: 1 != 2

======================================================================
ERROR: test_bar (test.test_example.TestClass.test_bar)
----------------------------------------------------------------------
Traceback (most recent call last):
  File "test.py", line 20, in test_bar
    raise ValueError("oops")
ValueError: oops

======================================================================
"""
        result = parse_results(_make_result(stdout))
        by_name = {t.name: t for t in result.tests}
        self.assertEqual(by_name["test_foo"].error_message, "AssertionError: 1 != 2")
        self.assertEqual(by_name["test_bar"].error_message, "ValueError: oops")

    def test_parse_directory_test_multiple_submodules(self):
        """Failures across submodule boundaries are all detected."""
        stdout = """\
Run 3 tests sequentially
0:00:00 [  1/3] test_asyncio.test_buffered_proto
test_ok (test.test_asyncio.test_buffered_proto.TestProto.test_ok) ... ok

----------------------------------------------------------------------
Ran 1 tests in 0.1s

OK

0:00:01 [  2/3] test_asyncio.test_events
test_create (test.test_asyncio.test_events.TestEvents.test_create) ... FAIL

----------------------------------------------------------------------
Ran 1 tests in 0.2s

FAILED (failures=1)

0:00:02 [  3/3] test_asyncio.test_tasks
test_gather (test.test_asyncio.test_tasks.TestTasks.test_gather) ... ERROR

----------------------------------------------------------------------
Ran 1 tests in 0.3s

FAILED (errors=1)

== Tests result: FAILURE ==
"""
        result = parse_results(_make_result(stdout))
        self.assertEqual(len(result.tests), 2)
        names = {t.name for t in result.tests}
        self.assertIn("test_create", names)
        self.assertIn("test_gather", names)
        self.assertEqual(result.tests_result, "FAILURE")

    def test_parse_multiline_test_with_docstring(self):
        """Two-line output (test_name + docstring ... RESULT) is handled."""
        stdout = """\
Run 3 tests sequentially
test_ok (test.test_example.TestClass.test_ok) ... ok
test_with_doc (test.test_example.TestClass.test_with_doc)
Test that something works ... ERROR
test_normal_fail (test.test_example.TestClass.test_normal_fail) ... FAIL
"""
        result = parse_results(_make_result(stdout))
        self.assertEqual(len(result.tests), 2)
        names = {t.name for t in result.tests}
        self.assertIn("test_with_doc", names)
        self.assertIn("test_normal_fail", names)
        test_doc = next(t for t in result.tests if t.name == "test_with_doc")
        self.assertEqual(test_doc.path, "test.test_example.TestClass.test_with_doc")
        self.assertEqual(test_doc.result, "error")


class TestPathToTestParts(unittest.TestCase):
    def test_simple_path(self):
        self.assertEqual(
            path_to_test_parts("test.test_foo.TestClass.test_method"),
            ["TestClass", "test_method"],
        )

    def test_nested_path(self):
        self.assertEqual(
            path_to_test_parts("test.test_foo.test_bar.TestClass.test_method"),
            ["TestClass", "test_method"],
        )


class TestCollectTestChanges(unittest.TestCase):
    def test_collect_failures_and_error_messages(self):
        """Failures and error messages are collected; empty messages are omitted."""
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

        self.assertEqual(
            failing, {("TestClass", "test_foo"), ("TestClass", "test_bar")}
        )
        self.assertEqual(successes, set())
        self.assertEqual(len(error_messages), 1)
        self.assertEqual(
            error_messages[("TestClass", "test_foo")], "AssertionError: 1 != 2"
        )

    def test_collect_unexpected_successes(self):
        results = TestResult()
        results.unexpected_successes = [
            Test(
                name="test_foo",
                path="test.test_example.TestClass.test_foo",
                result="unexpected_success",
            ),
        ]
        _, successes, _ = collect_test_changes(results)
        self.assertEqual(successes, {("TestClass", "test_foo")})

    def test_module_prefix_filtering(self):
        """Prefix filters with both short and 'test.' prefix formats."""
        results = TestResult()
        results.tests = [
            Test(name="test_foo", path="test_a.TestClass.test_foo", result="fail"),
            Test(
                name="test_bar",
                path="test.test_dataclasses.TestCase.test_bar",
                result="fail",
            ),
            Test(
                name="test_baz",
                path="test.test_other.TestOther.test_baz",
                result="fail",
            ),
        ]
        failing_a, _, _ = collect_test_changes(results, module_prefix="test_a.")
        self.assertEqual(failing_a, {("TestClass", "test_foo")})

        failing_dc, _, _ = collect_test_changes(
            results, module_prefix="test.test_dataclasses."
        )
        self.assertEqual(failing_dc, {("TestCase", "test_bar")})

    def test_collect_init_module_matching(self):
        """__init__.py tests match after stripping .__init__ from the prefix."""
        results = TestResult()
        results.tests = [
            Test(
                name="test_field_repr",
                path="test.test_dataclasses.TestCase.test_field_repr",
                result="fail",
            ),
        ]
        module_prefix = "test_dataclasses.__init__"
        if module_prefix.endswith(".__init__"):
            module_prefix = module_prefix[:-9]
        module_prefix = "test." + module_prefix + "."

        failing, _, _ = collect_test_changes(results, module_prefix=module_prefix)
        self.assertEqual(failing, {("TestCase", "test_field_repr")})


class TestExtractTestMethods(unittest.TestCase):
    def test_extract_methods(self):
        """Extracts from single and multiple classes."""
        code = """
class TestA(unittest.TestCase):
    def test_a(self):
        pass

class TestB(unittest.TestCase):
    def test_b(self):
        pass
"""
        methods = extract_test_methods(code)
        self.assertEqual(methods, {("TestA", "test_a"), ("TestB", "test_b")})

    def test_extract_syntax_error_returns_empty(self):
        self.assertEqual(extract_test_methods("this is not valid python {"), set())


class TestRemoveExpectedFailures(unittest.TestCase):
    def test_remove_comment_before(self):
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

    def test_remove_inline_comment(self):
        code = f"""import unittest

class TestFoo(unittest.TestCase):
    @unittest.expectedFailure  # {COMMENT}
    def test_one(self):
        pass
"""
        result = remove_expected_failures(code, {("TestFoo", "test_one")})
        self.assertNotIn("@unittest.expectedFailure", result)

    def test_remove_super_call_method(self):
        """Super-call-only override is removed entirely (sync)."""
        code = f"""import unittest

class TestFoo(unittest.TestCase):
    # {COMMENT}
    @unittest.expectedFailure
    def test_one(self):
        return super().test_one()
"""
        result = remove_expected_failures(code, {("TestFoo", "test_one")})
        self.assertNotIn("def test_one", result)

    def test_remove_async_super_call_override(self):
        """Super-call-only override is removed entirely (async)."""
        code = f"""import unittest

class BaseTest:
    async def test_async_one(self):
        pass

class TestChild(BaseTest, unittest.TestCase):
    # {COMMENT}
    @unittest.expectedFailure
    async def test_async_one(self):
        return await super().test_async_one()
"""
        result = remove_expected_failures(code, {("TestChild", "test_async_one")})
        self.assertNotIn("return await super().test_async_one()", result)
        self.assertNotIn("@unittest.expectedFailure", result)
        self.assertIn("class TestChild", result)
        self.assertIn("async def test_async_one(self):", result)

    def test_remove_with_comment_after(self):
        """Reason comment on the line after the decorator is also removed."""
        code = f"""import unittest

class TestFoo(unittest.TestCase):
    @unittest.expectedFailure  # {COMMENT}
    # RuntimeError: something went wrong
    def test_one(self):
        pass
"""
        result = remove_expected_failures(code, {("TestFoo", "test_one")})
        self.assertNotIn("@unittest.expectedFailure", result)
        self.assertNotIn("RuntimeError: something went wrong", result)
        self.assertIn("def test_one(self):", result)

    def test_no_removal_without_comment(self):
        """Decorators without our COMMENT marker are left untouched."""
        code = """import unittest

class TestFoo(unittest.TestCase):
    @unittest.expectedFailure
    def test_one(self):
        pass
"""
        result = remove_expected_failures(code, {("TestFoo", "test_one")})
        self.assertIn("@unittest.expectedFailure", result)


class TestStripReasonlessExpectedFailures(unittest.TestCase):
    def test_strip_reason_formats(self):
        """Strips both inline-comment and comment-before formats when no reason."""
        for label, code in [
            (
                "inline",
                f"""import unittest

class TestFoo(unittest.TestCase):
    @unittest.expectedFailure  # {COMMENT}
    def test_one(self):
        pass
""",
            ),
            (
                "comment-before",
                f"""import unittest

class TestFoo(unittest.TestCase):
    # {COMMENT}
    @unittest.expectedFailure
    def test_one(self):
        pass
""",
            ),
        ]:
            with self.subTest(label):
                result, stripped = strip_reasonless_expected_failures(code)
                self.assertNotIn("@unittest.expectedFailure", result)
                self.assertIn("def test_one(self):", result)
                self.assertEqual(stripped, {("TestFoo", "test_one")})

    def test_keep_with_reason(self):
        code = f"""import unittest

class TestFoo(unittest.TestCase):
    @unittest.expectedFailure  # {COMMENT}; AssertionError: 1 != 2
    def test_one(self):
        pass
"""
        result, stripped = strip_reasonless_expected_failures(code)
        self.assertIn("@unittest.expectedFailure", result)
        self.assertEqual(stripped, set())

    def test_strip_with_comment_after(self):
        """Old-format reason comment on the next line is also removed."""
        code = f"""import unittest

class TestFoo(unittest.TestCase):
    @unittest.expectedFailure  # {COMMENT}
    # RuntimeError: something went wrong
    def test_one(self):
        pass
"""
        result, stripped = strip_reasonless_expected_failures(code)
        self.assertNotIn("RuntimeError", result)
        self.assertIn("def test_one(self):", result)
        self.assertEqual(stripped, {("TestFoo", "test_one")})

    def test_strip_super_call_override(self):
        """Super-call overrides are removed entirely (both comment formats)."""
        for label, code in [
            (
                "comment-before",
                f"""import unittest

class _BaseTests:
    def test_foo(self):
        pass

class TestChild(_BaseTests, unittest.TestCase):
    # {COMMENT}
    @unittest.expectedFailure
    def test_foo(self):
        return super().test_foo()
""",
            ),
            (
                "inline",
                f"""import unittest

class _BaseTests:
    def test_foo(self):
        pass

class TestChild(_BaseTests, unittest.TestCase):
    @unittest.expectedFailure  # {COMMENT}
    def test_foo(self):
        return super().test_foo()
""",
            ),
        ]:
            with self.subTest(label):
                result, stripped = strip_reasonless_expected_failures(code)
                self.assertNotIn("return super().test_foo()", result)
                self.assertNotIn("@unittest.expectedFailure", result)
                self.assertEqual(stripped, {("TestChild", "test_foo")})
                self.assertIn("class _BaseTests:", result)

    def test_no_strip_without_comment(self):
        """Markers without our COMMENT are NOT stripped."""
        code = """import unittest

class TestFoo(unittest.TestCase):
    @unittest.expectedFailure
    def test_one(self):
        pass
"""
        result, stripped = strip_reasonless_expected_failures(code)
        self.assertIn("@unittest.expectedFailure", result)
        self.assertEqual(stripped, set())

    def test_mixed_with_and_without_reason(self):
        code = f"""import unittest

class TestFoo(unittest.TestCase):
    @unittest.expectedFailure  # {COMMENT}
    def test_no_reason(self):
        pass

    @unittest.expectedFailure  # {COMMENT}; has a reason
    def test_has_reason(self):
        pass
"""
        result, stripped = strip_reasonless_expected_failures(code)
        self.assertEqual(stripped, {("TestFoo", "test_no_reason")})
        self.assertIn("has a reason", result)
        self.assertEqual(result.count("@unittest.expectedFailure"), 1)


class TestExpandStrippedToChildren(unittest.TestCase):
    def test_parent_to_children(self):
        """Parent stripped → all/partial failing children returned."""
        stripped = {("Base", "test_foo")}
        all_children = {("ChildA", "test_foo"), ("ChildB", "test_foo")}

        # All children fail
        result = _expand_stripped_to_children(BASE_TWO_CHILDREN, stripped, all_children)
        self.assertEqual(result, all_children)

        # Only one child fails
        partial = {("ChildA", "test_foo")}
        result = _expand_stripped_to_children(BASE_TWO_CHILDREN, stripped, partial)
        self.assertEqual(result, partial)

    def test_direct_match(self):
        code = """import unittest

class TestFoo(unittest.TestCase):
    def test_one(self):
        pass
"""
        s = {("TestFoo", "test_one")}
        self.assertEqual(_expand_stripped_to_children(code, s, s), s)

    def test_child_with_own_override_excluded(self):
        stripped = {("Base", "test_foo")}
        all_failing = {("ChildA", "test_foo"), ("ChildB", "test_foo")}
        result = _expand_stripped_to_children(
            BASE_TWO_CHILDREN_ONE_OVERRIDE, stripped, all_failing
        )
        # ChildA inherits → included; ChildB has own method → excluded
        self.assertEqual(result, {("ChildA", "test_foo")})


class TestApplyTestChanges(unittest.TestCase):
    def test_apply_failing_tests(self):
        code = """import unittest

class TestFoo(unittest.TestCase):
    def test_one(self):
        pass
"""
        result = apply_test_changes(code, {("TestFoo", "test_one")}, set())
        self.assertIn("@unittest.expectedFailure", result)
        self.assertIn(COMMENT, result)

    def test_apply_removes_unexpected_success(self):
        code = f"""import unittest

class TestFoo(unittest.TestCase):
    # {COMMENT}
    @unittest.expectedFailure
    def test_one(self):
        pass
"""
        result = apply_test_changes(code, set(), {("TestFoo", "test_one")})
        self.assertNotIn("@unittest.expectedFailure", result)
        self.assertIn("def test_one(self):", result)

    def test_apply_both_changes(self):
        code = f"""import unittest

class TestFoo(unittest.TestCase):
    def test_one(self):
        pass

    # {COMMENT}
    @unittest.expectedFailure
    def test_two(self):
        pass
"""
        result = apply_test_changes(
            code, {("TestFoo", "test_one")}, {("TestFoo", "test_two")}
        )
        self.assertEqual(result.count("@unittest.expectedFailure"), 1)

    def test_apply_with_error_message(self):
        code = """import unittest

class TestFoo(unittest.TestCase):
    def test_one(self):
        pass
"""
        result = apply_test_changes(
            code,
            {("TestFoo", "test_one")},
            set(),
            {("TestFoo", "test_one"): "AssertionError: 1 != 2"},
        )
        self.assertIn("AssertionError: 1 != 2", result)
        self.assertIn(COMMENT, result)


class TestConsolidateToParent(unittest.TestCase):
    def test_all_children_fail_marks_parent_with_message(self):
        """All subclasses fail → marks parent; error message is transferred."""
        failing = {("ChildA", "test_foo"), ("ChildB", "test_foo")}
        error_messages = {("ChildA", "test_foo"): "RuntimeError: boom"}
        result = apply_test_changes(BASE_TWO_CHILDREN, failing, set(), error_messages)

        self.assertEqual(result.count("@unittest.expectedFailure"), 1)
        self.assertNotIn("return super()", result)
        self.assertIn("RuntimeError: boom", result)

    def test_partial_children_fail_marks_children(self):
        result = apply_test_changes(BASE_TWO_CHILDREN, {("ChildA", "test_foo")}, set())
        self.assertIn("return super().test_foo()", result)
        self.assertEqual(result.count("@unittest.expectedFailure"), 1)

    def test_child_with_own_override_not_consolidated(self):
        failing = {("ChildA", "test_foo"), ("ChildB", "test_foo")}
        result = apply_test_changes(BASE_TWO_CHILDREN_ONE_OVERRIDE, failing, set())
        self.assertEqual(result.count("@unittest.expectedFailure"), 2)

    def test_strip_then_consolidate_restores_parent_marker(self):
        """End-to-end: strip parent marker → child failures → re-mark on parent."""
        code = f"""import unittest

class _BaseTests:
    @unittest.expectedFailure  # {COMMENT}
    def test_foo(self):
        pass

class ChildA(_BaseTests, unittest.TestCase):
    pass

class ChildB(_BaseTests, unittest.TestCase):
    pass
"""
        stripped_code, stripped_tests = strip_reasonless_expected_failures(code)
        self.assertEqual(stripped_tests, {("_BaseTests", "test_foo")})

        all_failing = {("ChildA", "test_foo"), ("ChildB", "test_foo")}
        error_messages = {("ChildA", "test_foo"): "RuntimeError: boom"}

        to_remark = _expand_stripped_to_children(
            stripped_code, stripped_tests, all_failing
        )
        self.assertEqual(to_remark, all_failing)

        result = apply_test_changes(stripped_code, to_remark, set(), error_messages)
        self.assertIn("RuntimeError: boom", result)
        self.assertEqual(result.count("@unittest.expectedFailure"), 1)
        self.assertNotIn("return super()", result)


class TestSmartAutoMarkFiltering(unittest.TestCase):
    """Tests for smart auto-mark filtering (new tests vs regressions)."""

    @staticmethod
    def _filter(all_failing, original, current):
        new = current - original
        to_mark = {t for t in all_failing if t in new}
        return to_mark, all_failing - to_mark

    def test_new_vs_regression(self):
        """New failures are marked; existing (regression) failures are not."""
        original = {("TestFoo", "test_old1"), ("TestFoo", "test_old2")}
        current = original | {("TestFoo", "test_new1"), ("TestFoo", "test_new2")}
        all_failing = {("TestFoo", "test_old1"), ("TestFoo", "test_new1")}

        to_mark, regressions = self._filter(all_failing, original, current)
        self.assertEqual(to_mark, {("TestFoo", "test_new1")})
        self.assertEqual(regressions, {("TestFoo", "test_old1")})

        # Edge: all new → all marked
        to_mark, regressions = self._filter(all_failing, set(), current)
        self.assertEqual(to_mark, all_failing)
        self.assertEqual(regressions, set())

        # Edge: all old → nothing marked
        to_mark, regressions = self._filter(all_failing, current, current)
        self.assertEqual(to_mark, set())
        self.assertEqual(regressions, all_failing)

    def test_filters_across_classes(self):
        original = {("TestA", "test_a"), ("TestB", "test_b")}
        current = original | {("TestA", "test_new_a"), ("TestC", "test_c")}
        all_failing = {
            ("TestA", "test_a"),  # regression
            ("TestA", "test_new_a"),  # new
            ("TestC", "test_c"),  # new (new class)
        }
        to_mark, regressions = self._filter(all_failing, original, current)
        self.assertEqual(to_mark, {("TestA", "test_new_a"), ("TestC", "test_c")})
        self.assertEqual(regressions, {("TestA", "test_a")})


class TestIsSuperCallOnly(unittest.TestCase):
    @staticmethod
    def _parse_method(code):
        tree = ast.parse(code)
        for node in ast.walk(tree):
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                return node
        return None

    def test_sync(self):
        cases = [
            ("return super().test_one()", True),
            ("return super().test_two()", False),  # mismatched name
            ("pass", False),  # regular body
            ("x = 1\n        return super().test_one()", False),  # multiple stmts
        ]
        for body, expected in cases:
            with self.subTest(body=body):
                code = f"""
class Foo:
    def test_one(self):
        {body}
"""
                self.assertEqual(
                    _is_super_call_only(self._parse_method(code)), expected
                )

    def test_async(self):
        cases = [
            ("return await super().test_one()", True),
            ("return await super().test_two()", False),
            ("return super().test_one()", True),  # sync call in async method
        ]
        for body, expected in cases:
            with self.subTest(body=body):
                code = f"""
class Foo:
    async def test_one(self):
        {body}
"""
                self.assertEqual(
                    _is_super_call_only(self._parse_method(code)), expected
                )


class TestAutoMarkFileWithCrashedRun(unittest.TestCase):
    """auto_mark_file should process partial results when test runner crashes."""

    CRASHED_STDOUT = """\
Run 1 test sequentially in a single process
0:00:00 [1/1] test_example
test_foo (test.test_example.TestA.test_foo) ... FAIL
test_bar (test.test_example.TestA.test_bar) ... ok
======================================================================
FAIL: test_foo (test.test_example.TestA.test_foo)
----------------------------------------------------------------------
Traceback (most recent call last):
  File "test.py", line 10, in test_foo
    self.assertEqual(1, 2)
AssertionError: 1 != 2
"""

    def test_auto_mark_file_crashed_run(self):
        """auto_mark_file processes results even when tests_result is empty (crash)."""
        test_code = f"""import unittest

class TestA(unittest.TestCase):
    def test_foo(self):
        pass

    def test_bar(self):
        pass
"""
        with tempfile.TemporaryDirectory() as tmpdir:
            test_file = pathlib.Path(tmpdir) / "test_example.py"
            test_file.write_text(test_code)

            mock_result = TestResult()
            mock_result.tests_result = ""
            mock_result.tests = [
                Test(
                    name="test_foo",
                    path="test.test_example.TestA.test_foo",
                    result="fail",
                    error_message="AssertionError: 1 != 2",
                ),
            ]

            with mock.patch(
                "update_lib.cmd_auto_mark.run_test", return_value=mock_result
            ):
                added, removed, regressions = auto_mark_file(
                    test_file, mark_failure=True, verbose=False
                )

            self.assertEqual(added, 1)
            contents = test_file.read_text()
            self.assertIn("expectedFailure", contents)

    def test_auto_mark_file_no_results_at_all_raises(self):
        """auto_mark_file raises TestRunError when there are zero parsed results."""
        test_code = """import unittest

class TestA(unittest.TestCase):
    def test_foo(self):
        pass
"""
        with tempfile.TemporaryDirectory() as tmpdir:
            test_file = pathlib.Path(tmpdir) / "test_example.py"
            test_file.write_text(test_code)

            mock_result = TestResult()
            mock_result.tests_result = ""
            mock_result.tests = []
            mock_result.stdout = "some crash output"

            with mock.patch(
                "update_lib.cmd_auto_mark.run_test", return_value=mock_result
            ):
                with self.assertRaises(TestRunError):
                    auto_mark_file(test_file, verbose=False)


class TestAutoMarkDirectoryWithCrashedRun(unittest.TestCase):
    """auto_mark_directory should process partial results when test runner crashes."""

    def test_auto_mark_directory_crashed_run(self):
        """auto_mark_directory processes results even when tests_result is empty."""
        test_code = f"""import unittest

class TestA(unittest.TestCase):
    def test_foo(self):
        pass
"""
        with tempfile.TemporaryDirectory() as tmpdir:
            test_dir = pathlib.Path(tmpdir) / "test_example"
            test_dir.mkdir()
            test_file = test_dir / "test_sub.py"
            test_file.write_text(test_code)

            mock_result = TestResult()
            mock_result.tests_result = ""
            mock_result.tests = [
                Test(
                    name="test_foo",
                    path="test.test_example.test_sub.TestA.test_foo",
                    result="fail",
                    error_message="AssertionError: oops",
                ),
            ]

            with (
                mock.patch(
                    "update_lib.cmd_auto_mark.run_test", return_value=mock_result
                ),
                mock.patch(
                    "update_lib.cmd_auto_mark.get_test_module_name",
                    side_effect=lambda p: (
                        "test_example" if p == test_dir else "test_example.test_sub"
                    ),
                ),
            ):
                added, removed, regressions = auto_mark_directory(
                    test_dir, mark_failure=True, verbose=False
                )

            self.assertEqual(added, 1)
            contents = test_file.read_text()
            self.assertIn("expectedFailure", contents)

    def test_auto_mark_directory_no_results_raises(self):
        """auto_mark_directory raises TestRunError when zero results."""
        with tempfile.TemporaryDirectory() as tmpdir:
            test_dir = pathlib.Path(tmpdir) / "test_example"
            test_dir.mkdir()
            test_file = test_dir / "test_sub.py"
            test_file.write_text("import unittest\n")

            mock_result = TestResult()
            mock_result.tests_result = ""
            mock_result.tests = []
            mock_result.stdout = "crash"

            with (
                mock.patch(
                    "update_lib.cmd_auto_mark.run_test", return_value=mock_result
                ),
                mock.patch(
                    "update_lib.cmd_auto_mark.get_test_module_name",
                    return_value="test_example",
                ),
            ):
                with self.assertRaises(TestRunError):
                    auto_mark_directory(test_dir, verbose=False)


class TestAutoMarkFileRestoresOnCrash(unittest.TestCase):
    """Stripped markers must be restored when the test runner crashes."""

    def test_stripped_markers_restored_when_crash(self):
        """Markers stripped before run must be restored for unobserved tests on crash."""
        test_code = f"""\
import unittest

class TestA(unittest.TestCase):
    @unittest.expectedFailure  # {COMMENT}
    def test_foo(self):
        pass

    @unittest.expectedFailure  # {COMMENT}
    def test_bar(self):
        pass

    @unittest.expectedFailure  # {COMMENT}
    def test_baz(self):
        pass
"""
        with tempfile.TemporaryDirectory() as tmpdir:
            test_file = pathlib.Path(tmpdir) / "test_example.py"
            test_file.write_text(test_code)

            # Simulate a crashed run that only observed test_foo (failed)
            # test_bar and test_baz never ran due to crash
            mock_result = TestResult()
            mock_result.tests_result = ""  # no Tests result line (crash)
            mock_result.tests = [
                Test(
                    name="test_foo",
                    path="test.test_example.TestA.test_foo",
                    result="fail",
                    error_message="AssertionError: 1 != 2",
                ),
            ]

            with mock.patch(
                "update_lib.cmd_auto_mark.run_test", return_value=mock_result
            ):
                auto_mark_file(test_file, verbose=False)

            contents = test_file.read_text()
            # test_bar and test_baz were not observed — their markers must be restored
            self.assertIn("def test_bar", contents)
            self.assertIn("def test_baz", contents)
            # Count expectedFailure markers: all 3 should be present
            self.assertEqual(contents.count("expectedFailure"), 3, contents)

    def test_stripped_markers_removed_when_complete_run(self):
        """Markers are properly removed when the run completes normally."""
        test_code = f"""\
import unittest

class TestA(unittest.TestCase):
    @unittest.expectedFailure  # {COMMENT}
    def test_foo(self):
        pass

    @unittest.expectedFailure  # {COMMENT}
    def test_bar(self):
        pass
"""
        with tempfile.TemporaryDirectory() as tmpdir:
            test_file = pathlib.Path(tmpdir) / "test_example.py"
            test_file.write_text(test_code)

            # Simulate a complete run where test_foo fails but test_bar passes
            mock_result = TestResult()
            mock_result.tests_result = "FAILURE"  # normal completion
            mock_result.tests = [
                Test(
                    name="test_foo",
                    path="test.test_example.TestA.test_foo",
                    result="fail",
                    error_message="AssertionError",
                ),
            ]
            # test_bar passes → shows as unexpected success
            mock_result.unexpected_successes = [
                Test(
                    name="test_bar",
                    path="test.test_example.TestA.test_bar",
                    result="unexpected success",
                ),
            ]

            with mock.patch(
                "update_lib.cmd_auto_mark.run_test", return_value=mock_result
            ):
                auto_mark_file(test_file, verbose=False)

            contents = test_file.read_text()
            # test_foo should still have marker (re-added)
            self.assertEqual(contents.count("expectedFailure"), 1, contents)
            self.assertIn("def test_foo", contents)


class TestAutoMarkDirectoryRestoresOnCrash(unittest.TestCase):
    """Stripped markers must be restored for directory runs that crash."""

    def test_stripped_markers_restored_when_crash(self):
        test_code = f"""\
import unittest

class TestA(unittest.TestCase):
    @unittest.expectedFailure  # {COMMENT}
    def test_foo(self):
        pass

    @unittest.expectedFailure  # {COMMENT}
    def test_bar(self):
        pass
"""
        with tempfile.TemporaryDirectory() as tmpdir:
            test_dir = pathlib.Path(tmpdir) / "test_example"
            test_dir.mkdir()
            test_file = test_dir / "test_sub.py"
            test_file.write_text(test_code)

            mock_result = TestResult()
            mock_result.tests_result = ""  # crash
            mock_result.tests = [
                Test(
                    name="test_foo",
                    path="test.test_example.test_sub.TestA.test_foo",
                    result="fail",
                ),
            ]

            with (
                mock.patch(
                    "update_lib.cmd_auto_mark.run_test", return_value=mock_result
                ),
                mock.patch(
                    "update_lib.cmd_auto_mark.get_test_module_name",
                    side_effect=lambda p: (
                        "test_example" if p == test_dir else "test_example.test_sub"
                    ),
                ),
            ):
                auto_mark_directory(test_dir, verbose=False)

            contents = test_file.read_text()
            # Both markers must be present (unobserved test_bar restored)
            self.assertEqual(contents.count("expectedFailure"), 2, contents)


if __name__ == "__main__":
    unittest.main()
