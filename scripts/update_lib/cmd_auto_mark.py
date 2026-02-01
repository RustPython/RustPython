#!/usr/bin/env python
"""
Auto-mark test failures in Python test suite.

This module provides functions to:
- Run tests with RustPython and parse results
- Extract test names from test file paths
- Mark failing tests with @unittest.expectedFailure
- Remove expectedFailure from tests that now pass
"""

import ast
import pathlib
import re
import subprocess
import sys
from dataclasses import dataclass, field

sys.path.insert(0, str(pathlib.Path(__file__).parent.parent))

from update_lib import COMMENT, PatchSpec, UtMethod, apply_patches
from update_lib.file_utils import get_test_module_name


class TestRunError(Exception):
    """Raised when test run fails entirely (e.g., import error, crash)."""

    pass


@dataclass
class Test:
    name: str = ""
    path: str = ""
    result: str = ""
    error_message: str = ""


@dataclass
class TestResult:
    tests_result: str = ""
    tests: list[Test] = field(default_factory=list)
    unexpected_successes: list[Test] = field(default_factory=list)
    stdout: str = ""


def run_test(test_name: str, skip_build: bool = False) -> TestResult:
    """
    Run a test with RustPython and return parsed results.

    Args:
        test_name: Test module name (e.g., "test_foo" or "test_ctypes.test_bar")
        skip_build: If True, use pre-built binary instead of cargo run

    Returns:
        TestResult with parsed test results
    """
    if skip_build:
        cmd = ["./target/release/rustpython"]
        if sys.platform == "win32":
            cmd = ["./target/release/rustpython.exe"]
    else:
        cmd = ["cargo", "run", "--release", "--"]

    result = subprocess.run(
        cmd + ["-m", "test", "-v", "-u", "all", "--slowest", test_name],
        stdout=subprocess.PIPE,  # Capture stdout for parsing
        stderr=None,  # Let stderr pass through to terminal
        text=True,
    )
    return parse_results(result)


def _try_parse_test_info(test_info: str) -> tuple[str, str] | None:
    """Try to extract (name, path) from 'test_name (path)' or 'test_name (path) [subtest]'."""
    first_space = test_info.find(" ")
    if first_space > 0:
        name = test_info[:first_space]
        rest = test_info[first_space:].strip()
        if rest.startswith("("):
            end_paren = rest.find(")")
            if end_paren > 0:
                return name, rest[1:end_paren]
    return None


def parse_results(result: subprocess.CompletedProcess) -> TestResult:
    """Parse subprocess result into TestResult."""
    lines = result.stdout.splitlines()
    test_results = TestResult()
    test_results.stdout = result.stdout
    in_test_results = False
    # For multiline format: "test_name (path)\ndocstring ... RESULT"
    pending_test_info = None

    for line in lines:
        if re.search(r"Run \d+ tests? sequentially", line):
            in_test_results = True
        elif "== Tests result: " in line:
            in_test_results = False

        if in_test_results and " ... " in line:
            stripped = line.strip()
            # Skip lines that don't look like test results
            if stripped.startswith("tests") or stripped.startswith("["):
                pending_test_info = None
                continue
            # Parse: "test_name (path) [subtest] ... RESULT"
            parts = stripped.split(" ... ")
            if len(parts) >= 2:
                test_info = parts[0]
                result_str = parts[-1].lower()
                # Only process FAIL or ERROR
                if result_str not in ("fail", "error"):
                    pending_test_info = None
                    continue
                # Try parsing from this line (single-line format)
                parsed = _try_parse_test_info(test_info)
                if not parsed and pending_test_info:
                    # Multiline format: previous line had test_name (path)
                    parsed = _try_parse_test_info(pending_test_info)
                if parsed:
                    test = Test()
                    test.name, test.path = parsed
                    test.result = result_str
                    test_results.tests.append(test)
                pending_test_info = None

        elif in_test_results:
            # Track test info for multiline format:
            #   test_name (path)
            #   docstring ... RESULT
            stripped = line.strip()
            if (
                stripped
                and "(" in stripped
                and stripped.endswith(")")
                and ":" not in stripped.split("(")[0]
            ):
                pending_test_info = stripped
            else:
                pending_test_info = None

            # Also check for Tests result on non-" ... " lines
            if "== Tests result: " in line:
                res = line.split("== Tests result: ")[1]
                res = res.split(" ")[0]
                test_results.tests_result = res

        elif "== Tests result: " in line:
            res = line.split("== Tests result: ")[1]
            res = res.split(" ")[0]
            test_results.tests_result = res

        # Parse: "UNEXPECTED SUCCESS: test_name (path)"
        if line.startswith("UNEXPECTED SUCCESS: "):
            rest = line[len("UNEXPECTED SUCCESS: ") :]
            # Format: "test_name (path)"
            first_space = rest.find(" ")
            if first_space > 0:
                test = Test()
                test.name = rest[:first_space]
                path_part = rest[first_space:].strip()
                if path_part.startswith("(") and path_part.endswith(")"):
                    test.path = path_part[1:-1]
                    test.result = "unexpected_success"
                    test_results.unexpected_successes.append(test)

    # Parse error details to extract error messages
    _parse_error_details(test_results, lines)

    return test_results


def _parse_error_details(test_results: TestResult, lines: list[str]) -> None:
    """Parse error details section to extract error messages for each test."""
    # Build a lookup dict for tests by (name, path)
    test_lookup: dict[tuple[str, str], Test] = {}
    for test in test_results.tests:
        test_lookup[(test.name, test.path)] = test

    # Parse error detail blocks
    # Format:
    # ======================================================================
    # FAIL: test_name (path)
    # ----------------------------------------------------------------------
    # Traceback (most recent call last):
    #   ...
    # AssertionError: message
    #
    # ======================================================================
    i = 0
    while i < len(lines):
        line = lines[i]
        # Look for FAIL: or ERROR: header
        if line.startswith(("FAIL: ", "ERROR: ")):
            # Parse: "FAIL: test_name (path)" or "ERROR: test_name (path)"
            header = line.split(": ", 1)[1] if ": " in line else ""
            first_space = header.find(" ")
            if first_space > 0:
                test_name = header[:first_space]
                path_part = header[first_space:].strip()
                if path_part.startswith("(") and path_part.endswith(")"):
                    test_path = path_part[1:-1]

                    # Find the last non-empty line before the next separator or end
                    error_lines = []
                    i += 1
                    # Skip the separator line
                    if i < len(lines) and lines[i].startswith("-----"):
                        i += 1

                    # Collect lines until the next separator or end
                    while i < len(lines):
                        current = lines[i]
                        if current.startswith("=====") or current.startswith("-----"):
                            break
                        error_lines.append(current)
                        i += 1

                    # Find the last non-empty line (the error message)
                    error_message = ""
                    for err_line in reversed(error_lines):
                        stripped = err_line.strip()
                        if stripped:
                            error_message = stripped
                            break

                    # Update the test with the error message
                    if (test_name, test_path) in test_lookup:
                        test_lookup[
                            (test_name, test_path)
                        ].error_message = error_message

                    continue
        i += 1


def path_to_test_parts(path: str) -> list[str]:
    """
    Extract [ClassName, method_name] from test path.

    Args:
        path: Test path like "test.module_name.ClassName.test_method"

    Returns:
        [ClassName, method_name] - last 2 elements
    """
    parts = path.split(".")
    return parts[-2:]


def _expand_stripped_to_children(
    contents: str,
    stripped_tests: set[tuple[str, str]],
    all_failing_tests: set[tuple[str, str]],
) -> set[tuple[str, str]]:
    """Find child-class failures that correspond to stripped parent-class markers.

    When ``strip_reasonless_expected_failures`` removes a marker from a parent
    (mixin) class, test failures are reported against the concrete subclasses,
    not the parent itself.  This function maps those child failures back so
    they get re-marked (and later consolidated to the parent by
    ``_consolidate_to_parent``).

    Returns the set of ``(class, method)`` pairs from *all_failing_tests* that
    should be re-marked.
    """
    # Direct matches (stripped test itself is a concrete TestCase)
    result = stripped_tests & all_failing_tests

    unmatched = stripped_tests - all_failing_tests
    if not unmatched:
        return result

    tree = ast.parse(contents)
    class_bases, class_methods = _build_inheritance_info(tree)

    for parent_cls, method_name in unmatched:
        if method_name not in class_methods.get(parent_cls, set()):
            continue
        for cls in _find_all_inheritors(
            parent_cls, method_name, class_bases, class_methods
        ):
            if (cls, method_name) in all_failing_tests:
                result.add((cls, method_name))

    return result


def _consolidate_to_parent(
    contents: str,
    failing_tests: set[tuple[str, str]],
    error_messages: dict[tuple[str, str], str] | None = None,
) -> tuple[set[tuple[str, str]], dict[tuple[str, str], str] | None]:
    """Move failures to the parent class when ALL inheritors fail.

    If every concrete subclass that inherits a method from a parent class
    appears in *failing_tests*, replace those per-subclass entries with a
    single entry on the parent.  This avoids creating redundant super-call
    overrides in every child.

    Returns:
        (consolidated_failing_tests, consolidated_error_messages)
    """
    tree = ast.parse(contents)
    class_bases, class_methods = _build_inheritance_info(tree)

    # Group by (defining_parent, method) → set of failing children
    from collections import defaultdict

    groups: dict[tuple[str, str], set[str]] = defaultdict(set)
    for class_name, method_name in failing_tests:
        defining = _find_method_definition(
            class_name, method_name, class_bases, class_methods
        )
        if defining and defining != class_name:
            groups[(defining, method_name)].add(class_name)

    if not groups:
        return failing_tests, error_messages

    result = set(failing_tests)
    new_error_messages = dict(error_messages) if error_messages else {}

    for (parent, method_name), failing_children in groups.items():
        all_inheritors = _find_all_inheritors(
            parent, method_name, class_bases, class_methods
        )

        if all_inheritors and failing_children >= all_inheritors:
            # All inheritors fail → mark on parent instead
            children_keys = {(child, method_name) for child in failing_children}
            result -= children_keys
            result.add((parent, method_name))
            # Pick any child's error message for the parent
            if new_error_messages:
                for child in failing_children:
                    msg = new_error_messages.pop((child, method_name), "")
                    if msg:
                        new_error_messages[(parent, method_name)] = msg

    return result, new_error_messages or error_messages


def build_patches(
    test_parts_set: set[tuple[str, str]],
    error_messages: dict[tuple[str, str], str] | None = None,
) -> dict:
    """Convert failing tests to patch format."""
    patches = {}
    error_messages = error_messages or {}
    for class_name, method_name in test_parts_set:
        if class_name not in patches:
            patches[class_name] = {}
        reason = error_messages.get((class_name, method_name), "")
        patches[class_name][method_name] = [
            PatchSpec(UtMethod.ExpectedFailure, None, reason)
        ]
    return patches


def _is_super_call_only(func_node: ast.FunctionDef | ast.AsyncFunctionDef) -> bool:
    """Check if the method body is just 'return super().method_name()' or 'return await super().method_name()'."""
    if len(func_node.body) != 1:
        return False
    stmt = func_node.body[0]
    if not isinstance(stmt, ast.Return) or stmt.value is None:
        return False
    call = stmt.value
    # Unwrap await for async methods
    if isinstance(call, ast.Await):
        call = call.value
    if not isinstance(call, ast.Call):
        return False
    if not isinstance(call.func, ast.Attribute):
        return False
    # Verify the method name matches
    if call.func.attr != func_node.name:
        return False
    super_call = call.func.value
    if not isinstance(super_call, ast.Call):
        return False
    if not isinstance(super_call.func, ast.Name) or super_call.func.id != "super":
        return False
    return True


def _method_removal_range(
    func_node: ast.FunctionDef | ast.AsyncFunctionDef, lines: list[str]
) -> range:
    """Line range covering an entire method including decorators and a preceding COMMENT line."""
    first = (
        func_node.decorator_list[0].lineno - 1
        if func_node.decorator_list
        else func_node.lineno - 1
    )
    if (
        first > 0
        and lines[first - 1].strip().startswith("#")
        and COMMENT in lines[first - 1]
    ):
        first -= 1
    return range(first, func_node.end_lineno)


def _build_inheritance_info(tree: ast.Module) -> tuple[dict, dict]:
    """
    Build inheritance information from AST.

    Returns:
        class_bases: dict[str, list[str]] - parent classes for each class
        class_methods: dict[str, set[str]] - methods directly defined in each class
    """
    all_classes = {
        node.name for node in ast.walk(tree) if isinstance(node, ast.ClassDef)
    }
    class_bases = {}
    class_methods = {}

    for node in ast.walk(tree):
        if isinstance(node, ast.ClassDef):
            bases = [
                base.id
                for base in node.bases
                if isinstance(base, ast.Name) and base.id in all_classes
            ]
            class_bases[node.name] = bases
            methods = {
                item.name
                for item in node.body
                if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef))
            }
            class_methods[node.name] = methods

    return class_bases, class_methods


def _find_method_definition(
    class_name: str, method_name: str, class_bases: dict, class_methods: dict
) -> str | None:
    """Find the class where a method is actually defined (BFS)."""
    if method_name in class_methods.get(class_name, set()):
        return class_name

    visited = set()
    queue = list(class_bases.get(class_name, []))

    while queue:
        current = queue.pop(0)
        if current in visited:
            continue
        visited.add(current)

        if method_name in class_methods.get(current, set()):
            return current
        queue.extend(class_bases.get(current, []))

    return None


def _find_all_inheritors(
    parent: str, method_name: str, class_bases: dict, class_methods: dict
) -> set[str]:
    """Find all classes that inherit *method_name* from *parent* (not overriding it)."""
    return {
        cls
        for cls in class_bases
        if cls != parent
        and method_name not in class_methods.get(cls, set())
        and _find_method_definition(cls, method_name, class_bases, class_methods)
        == parent
    }


def remove_expected_failures(
    contents: str, tests_to_remove: set[tuple[str, str]]
) -> str:
    """Remove @unittest.expectedFailure decorators from tests that now pass."""
    if not tests_to_remove:
        return contents

    tree = ast.parse(contents)
    lines = contents.splitlines()
    lines_to_remove = set()

    class_bases, class_methods = _build_inheritance_info(tree)

    resolved_tests = set()
    for class_name, method_name in tests_to_remove:
        defining_class = _find_method_definition(
            class_name, method_name, class_bases, class_methods
        )
        if defining_class:
            resolved_tests.add((defining_class, method_name))

    for node in ast.walk(tree):
        if not isinstance(node, ast.ClassDef):
            continue
        class_name = node.name
        for item in node.body:
            if not isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)):
                continue
            method_name = item.name
            if (class_name, method_name) not in resolved_tests:
                continue

            remove_entire_method = _is_super_call_only(item)

            if remove_entire_method:
                lines_to_remove.update(_method_removal_range(item, lines))
            else:
                for dec in item.decorator_list:
                    dec_line = dec.lineno - 1
                    line_content = lines[dec_line]

                    if "expectedFailure" not in line_content:
                        continue

                    has_comment_on_line = COMMENT in line_content
                    has_comment_before = (
                        dec_line > 0
                        and lines[dec_line - 1].strip().startswith("#")
                        and COMMENT in lines[dec_line - 1]
                    )
                    has_comment_after = (
                        dec_line + 1 < len(lines)
                        and lines[dec_line + 1].strip().startswith("#")
                        and COMMENT not in lines[dec_line + 1]
                    )

                    if has_comment_on_line or has_comment_before:
                        lines_to_remove.add(dec_line)
                        if has_comment_before:
                            lines_to_remove.add(dec_line - 1)
                        if has_comment_after and has_comment_on_line:
                            lines_to_remove.add(dec_line + 1)

    for line_idx in sorted(lines_to_remove, reverse=True):
        del lines[line_idx]

    return "\n".join(lines) + "\n" if lines else ""


def collect_test_changes(
    results: TestResult,
    module_prefix: str | None = None,
) -> tuple[set[tuple[str, str]], set[tuple[str, str]], dict[tuple[str, str], str]]:
    """
    Collect failing tests and unexpected successes from test results.

    Args:
        results: TestResult from run_test()
        module_prefix: If set, only collect tests whose path starts with this prefix

    Returns:
        (failing_tests, unexpected_successes, error_messages)
        - failing_tests: set of (class_name, method_name) tuples
        - unexpected_successes: set of (class_name, method_name) tuples
        - error_messages: dict mapping (class_name, method_name) to error message
    """
    failing_tests = set()
    error_messages: dict[tuple[str, str], str] = {}
    for test in results.tests:
        if test.result in ("fail", "error"):
            if module_prefix and not test.path.startswith(module_prefix):
                continue
            test_parts = path_to_test_parts(test.path)
            if len(test_parts) == 2:
                key = tuple(test_parts)
                failing_tests.add(key)
                if test.error_message:
                    error_messages[key] = test.error_message

    unexpected_successes = set()
    for test in results.unexpected_successes:
        if module_prefix and not test.path.startswith(module_prefix):
            continue
        test_parts = path_to_test_parts(test.path)
        if len(test_parts) == 2:
            unexpected_successes.add(tuple(test_parts))

    return failing_tests, unexpected_successes, error_messages


def apply_test_changes(
    contents: str,
    failing_tests: set[tuple[str, str]],
    unexpected_successes: set[tuple[str, str]],
    error_messages: dict[tuple[str, str], str] | None = None,
) -> str:
    """
    Apply test changes to content.

    Args:
        contents: File content
        failing_tests: Set of (class_name, method_name) to mark as expectedFailure
        unexpected_successes: Set of (class_name, method_name) to remove expectedFailure
        error_messages: Dict mapping (class_name, method_name) to error message

    Returns:
        Modified content
    """
    if unexpected_successes:
        contents = remove_expected_failures(contents, unexpected_successes)

    if failing_tests:
        failing_tests, error_messages = _consolidate_to_parent(
            contents, failing_tests, error_messages
        )
        patches = build_patches(failing_tests, error_messages)
        contents = apply_patches(contents, patches)

    return contents


def strip_reasonless_expected_failures(
    contents: str,
) -> tuple[str, set[tuple[str, str]]]:
    """Strip @expectedFailure decorators that have no failure reason.

    Markers like ``@unittest.expectedFailure  # TODO: RUSTPYTHON`` (without a
    reason after the semicolon) are removed so the tests fail normally during
    the next test run and error messages can be captured.

    Returns:
        (modified_contents, stripped_tests) where stripped_tests is a set of
        (class_name, method_name) tuples whose markers were removed.
    """
    tree = ast.parse(contents)
    lines = contents.splitlines()
    stripped_tests: set[tuple[str, str]] = set()
    lines_to_remove: set[int] = set()

    for node in ast.walk(tree):
        if not isinstance(node, ast.ClassDef):
            continue
        for item in node.body:
            if not isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)):
                continue
            for dec in item.decorator_list:
                dec_line = dec.lineno - 1
                line_content = lines[dec_line]

                if "expectedFailure" not in line_content:
                    continue

                has_comment_on_line = COMMENT in line_content
                has_comment_before = (
                    dec_line > 0
                    and lines[dec_line - 1].strip().startswith("#")
                    and COMMENT in lines[dec_line - 1]
                )

                if not has_comment_on_line and not has_comment_before:
                    continue  # not our marker

                # Check if there's a reason (on either the decorator or before)
                for check_line in (
                    line_content,
                    lines[dec_line - 1] if has_comment_before else "",
                ):
                    match = re.search(rf"{COMMENT}(.*)", check_line)
                    if match and match.group(1).strip(";:, "):
                        break  # has a reason, keep it
                else:
                    # No reason found — strip this decorator
                    stripped_tests.add((node.name, item.name))

                    if _is_super_call_only(item):
                        # Remove entire super-call override (the method
                        # exists only to apply the decorator; without it
                        # the override is pointless and blocks parent
                        # consolidation)
                        lines_to_remove.update(_method_removal_range(item, lines))
                    else:
                        lines_to_remove.add(dec_line)

                        if has_comment_before:
                            lines_to_remove.add(dec_line - 1)

                        # Also remove a reason-comment on the line after (old format)
                        if (
                            has_comment_on_line
                            and dec_line + 1 < len(lines)
                            and lines[dec_line + 1].strip().startswith("#")
                            and COMMENT not in lines[dec_line + 1]
                        ):
                            lines_to_remove.add(dec_line + 1)

    if not lines_to_remove:
        return contents, stripped_tests

    for idx in sorted(lines_to_remove, reverse=True):
        del lines[idx]

    return "\n".join(lines) + "\n" if lines else "", stripped_tests


def extract_test_methods(contents: str) -> set[tuple[str, str]]:
    """
    Extract all test method names from file contents.

    Returns:
        Set of (class_name, method_name) tuples
    """
    from update_lib.file_utils import safe_parse_ast
    from update_lib.patch_spec import iter_tests

    tree = safe_parse_ast(contents)
    if tree is None:
        return set()

    return {(cls_node.name, fn_node.name) for cls_node, fn_node in iter_tests(tree)}


def auto_mark_file(
    test_path: pathlib.Path,
    mark_failure: bool = False,
    verbose: bool = True,
    original_methods: set[tuple[str, str]] | None = None,
    skip_build: bool = False,
) -> tuple[int, int, int]:
    """
    Run tests and auto-mark failures in a test file.

    Args:
        test_path: Path to the test file
        mark_failure: If True, add @expectedFailure to ALL failing tests
        verbose: Print progress messages
        original_methods: If provided, only auto-mark failures for NEW methods
                          (methods not in original_methods) even without mark_failure.
                          Failures in existing methods are treated as regressions.

    Returns:
        (num_failures_added, num_successes_removed, num_regressions)
    """
    test_path = pathlib.Path(test_path).resolve()
    if not test_path.exists():
        raise FileNotFoundError(f"File not found: {test_path}")

    # Strip reason-less markers so those tests fail normally and we capture
    # their error messages during the test run.
    contents = test_path.read_text(encoding="utf-8")
    contents, stripped_tests = strip_reasonless_expected_failures(contents)
    if stripped_tests:
        test_path.write_text(contents, encoding="utf-8")

    test_name = get_test_module_name(test_path)
    if verbose:
        print(f"Running test: {test_name}")

    results = run_test(test_name, skip_build=skip_build)

    # Check if test run failed entirely (e.g., import error, crash)
    if not results.tests_result:
        raise TestRunError(
            f"Test run failed for {test_name}. "
            f"Output: {results.stdout[-500:] if results.stdout else '(no output)'}"
        )

    contents = test_path.read_text(encoding="utf-8")

    all_failing_tests, unexpected_successes, error_messages = collect_test_changes(
        results
    )

    # Determine which failures to mark
    if mark_failure:
        failing_tests = all_failing_tests
    elif original_methods is not None:
        # Smart mode: only mark NEW test failures (not regressions)
        current_methods = extract_test_methods(contents)
        new_methods = current_methods - original_methods
        failing_tests = {t for t in all_failing_tests if t in new_methods}
    else:
        failing_tests = set()

    # Re-mark stripped tests that still fail (to restore markers with reasons).
    # Uses inheritance expansion: if a parent marker was stripped, child
    # failures are included so _consolidate_to_parent can re-mark the parent.
    failing_tests |= _expand_stripped_to_children(
        contents, stripped_tests, all_failing_tests
    )

    regressions = all_failing_tests - failing_tests

    if verbose:
        for class_name, method_name in failing_tests:
            label = "(new test)" if original_methods is not None else ""
            err_msg = error_messages.get((class_name, method_name), "")
            err_hint = f" - {err_msg}" if err_msg else ""
            print(
                f"Marking as failing {label}: {class_name}.{method_name}{err_hint}".replace(
                    "  ", " "
                )
            )
        for class_name, method_name in unexpected_successes:
            print(f"Removing expectedFailure: {class_name}.{method_name}")

    contents = apply_test_changes(
        contents, failing_tests, unexpected_successes, error_messages
    )

    if failing_tests or unexpected_successes:
        test_path.write_text(contents, encoding="utf-8")

    # Show hints about unmarked failures
    if verbose:
        unmarked_failures = all_failing_tests - failing_tests
        if unmarked_failures:
            print(
                f"Hint: {len(unmarked_failures)} failing tests can be marked with --mark-failure; "
                "but review first and do not blindly mark them all"
            )
            for class_name, method_name in sorted(unmarked_failures):
                err_msg = error_messages.get((class_name, method_name), "")
                err_hint = f" - {err_msg}" if err_msg else ""
                print(f"  {class_name}.{method_name}{err_hint}")

    return len(failing_tests), len(unexpected_successes), len(regressions)


def auto_mark_directory(
    test_dir: pathlib.Path,
    mark_failure: bool = False,
    verbose: bool = True,
    original_methods_per_file: dict[pathlib.Path, set[tuple[str, str]]] | None = None,
    skip_build: bool = False,
) -> tuple[int, int, int]:
    """
    Run tests and auto-mark failures in a test directory.

    Runs the test once for the whole directory, then applies results to each file.

    Args:
        test_dir: Path to the test directory
        mark_failure: If True, add @expectedFailure to ALL failing tests
        verbose: Print progress messages
        original_methods_per_file: If provided, only auto-mark failures for NEW methods
                                   even without mark_failure. Dict maps file path to
                                   set of (class_name, method_name) tuples.

    Returns:
        (num_failures_added, num_successes_removed, num_regressions)
    """
    test_dir = pathlib.Path(test_dir).resolve()
    if not test_dir.exists():
        raise FileNotFoundError(f"Directory not found: {test_dir}")
    if not test_dir.is_dir():
        raise ValueError(f"Not a directory: {test_dir}")

    # Get all .py files in directory
    test_files = sorted(test_dir.glob("**/*.py"))

    # Strip reason-less markers from ALL files before running tests so those
    # tests fail normally and we capture their error messages.
    stripped_per_file: dict[pathlib.Path, set[tuple[str, str]]] = {}
    for test_file in test_files:
        contents = test_file.read_text(encoding="utf-8")
        contents, stripped = strip_reasonless_expected_failures(contents)
        if stripped:
            test_file.write_text(contents, encoding="utf-8")
            stripped_per_file[test_file] = stripped

    test_name = get_test_module_name(test_dir)
    if verbose:
        print(f"Running test: {test_name}")

    results = run_test(test_name, skip_build=skip_build)

    # Check if test run failed entirely (e.g., import error, crash)
    if not results.tests_result:
        raise TestRunError(
            f"Test run failed for {test_name}. "
            f"Output: {results.stdout[-500:] if results.stdout else '(no output)'}"
        )

    total_added = 0
    total_removed = 0
    total_regressions = 0
    all_regressions: list[tuple[str, str, str, str]] = []

    for test_file in test_files:
        # Get module prefix for this file (e.g., "test_inspect.test_inspect")
        module_prefix = get_test_module_name(test_file)
        # For __init__.py, the test path doesn't include "__init__"
        if module_prefix.endswith(".__init__"):
            module_prefix = module_prefix[:-9]  # Remove ".__init__"

        all_failing_tests, unexpected_successes, error_messages = collect_test_changes(
            results, module_prefix="test." + module_prefix + "."
        )

        # Determine which failures to mark
        if mark_failure:
            failing_tests = all_failing_tests
        elif original_methods_per_file is not None:
            # Smart mode: only mark NEW test failures
            contents = test_file.read_text(encoding="utf-8")
            current_methods = extract_test_methods(contents)
            original_methods = original_methods_per_file.get(test_file, set())
            new_methods = current_methods - original_methods
            failing_tests = {t for t in all_failing_tests if t in new_methods}
        else:
            failing_tests = set()

        # Re-mark stripped tests that still fail (restore markers with reasons).
        # Uses inheritance expansion for parent→child mapping.
        stripped = stripped_per_file.get(test_file, set())
        if stripped:
            file_contents = test_file.read_text(encoding="utf-8")
            failing_tests |= _expand_stripped_to_children(
                file_contents, stripped, all_failing_tests
            )

        regressions = all_failing_tests - failing_tests

        if failing_tests or unexpected_successes:
            if verbose:
                for class_name, method_name in failing_tests:
                    label = (
                        "(new test)" if original_methods_per_file is not None else ""
                    )
                    err_msg = error_messages.get((class_name, method_name), "")
                    err_hint = f" - {err_msg}" if err_msg else ""
                    print(
                        f"  {test_file.name}: Marking as failing {label}: {class_name}.{method_name}{err_hint}".replace(
                            "  :", ":"
                        )
                    )
                for class_name, method_name in unexpected_successes:
                    print(
                        f"  {test_file.name}: Removing expectedFailure: {class_name}.{method_name}"
                    )

            contents = test_file.read_text(encoding="utf-8")
            contents = apply_test_changes(
                contents, failing_tests, unexpected_successes, error_messages
            )
            test_file.write_text(contents, encoding="utf-8")

        # Collect regressions with error messages for later reporting
        for class_name, method_name in regressions:
            err_msg = error_messages.get((class_name, method_name), "")
            all_regressions.append((test_file.name, class_name, method_name, err_msg))

        total_added += len(failing_tests)
        total_removed += len(unexpected_successes)
        total_regressions += len(regressions)

    # Show hints about unmarked failures
    if verbose and total_regressions > 0:
        print(
            f"Hint: {total_regressions} failing tests can be marked with --mark-failure; "
            "but review first and do not blindly mark them all"
        )
        for file_name, class_name, method_name, err_msg in sorted(all_regressions):
            err_hint = f" - {err_msg}" if err_msg else ""
            print(f"  {file_name}: {class_name}.{method_name}{err_hint}")

    return total_added, total_removed, total_regressions


def main(argv: list[str] | None = None) -> int:
    import argparse

    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "path",
        type=pathlib.Path,
        help="Path to test file or directory (e.g., Lib/test/test_foo.py or Lib/test/test_foo/)",
    )
    parser.add_argument(
        "--mark-failure",
        action="store_true",
        help="Also add @expectedFailure to failing tests (default: only remove unexpected successes)",
    )
    parser.add_argument(
        "--build",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Build with cargo (default: enabled)",
    )

    args = parser.parse_args(argv)

    try:
        if args.path.is_dir():
            num_added, num_removed, _ = auto_mark_directory(
                args.path, mark_failure=args.mark_failure, skip_build=not args.build
            )
        else:
            num_added, num_removed, _ = auto_mark_file(
                args.path, mark_failure=args.mark_failure, skip_build=not args.build
            )
        if args.mark_failure:
            print(f"Added expectedFailure to {num_added} tests")
        print(f"Removed expectedFailure from {num_removed} tests")
        return 0
    except (FileNotFoundError, ValueError) as e:
        print(f"Error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
