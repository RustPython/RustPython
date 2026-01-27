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


def parse_results(result: subprocess.CompletedProcess) -> TestResult:
    """Parse subprocess result into TestResult."""
    lines = result.stdout.splitlines()
    test_results = TestResult()
    test_results.stdout = result.stdout
    in_test_results = False

    for line in lines:
        if re.search(r"Run \d+ tests? sequentially", line):
            in_test_results = True
        elif line.startswith("-----------"):
            in_test_results = False

        if in_test_results and " ... " in line:
            line = line.strip()
            # Skip lines that don't look like test results
            if line.startswith("tests") or line.startswith("["):
                continue
            # Parse: "test_name (path) [subtest] ... RESULT"
            parts = line.split(" ... ")
            if len(parts) >= 2:
                test_info = parts[0]
                result_str = parts[-1].lower()
                # Only process FAIL or ERROR
                if result_str not in ("fail", "error"):
                    continue
                # Extract test name (first word)
                first_space = test_info.find(" ")
                if first_space > 0:
                    test = Test()
                    test.name = test_info[:first_space]
                    # Extract path from (path)
                    rest = test_info[first_space:].strip()
                    if rest.startswith("("):
                        end_paren = rest.find(")")
                        if end_paren > 0:
                            test.path = rest[1:end_paren]
                            test.result = result_str
                            test_results.tests.append(test)

        elif "== Tests result: " in line:
            res = line.split("== Tests result: ")[1]
            res = res.split(" ")[0]
            test_results.tests_result = res

        # Parse: "UNEXPECTED SUCCESS: test_name (path)"
        elif line.startswith("UNEXPECTED SUCCESS: "):
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
    """Check if the method body is just 'return super().method_name()'."""
    if len(func_node.body) != 1:
        return False
    stmt = func_node.body[0]
    if not isinstance(stmt, ast.Return) or stmt.value is None:
        return False
    call = stmt.value
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
                first_line = item.lineno - 1
                if item.decorator_list:
                    first_line = item.decorator_list[0].lineno - 1
                if first_line > 0:
                    prev_line = lines[first_line - 1].strip()
                    if prev_line.startswith("#") and COMMENT in prev_line:
                        first_line -= 1
                for i in range(first_line, item.end_lineno):
                    lines_to_remove.add(i)
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

                    if has_comment_on_line or has_comment_before:
                        lines_to_remove.add(dec_line)
                        if has_comment_before:
                            lines_to_remove.add(dec_line - 1)

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
        patches = build_patches(failing_tests, error_messages)
        contents = apply_patches(contents, patches)

    return contents


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

    # Get all .py files in directory
    test_files = sorted(test_dir.glob("**/*.py"))

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
