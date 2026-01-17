"""
An automated script to mark failures in python test suite.
It adds @unittest.expectedFailure to the test functions that are failing in RustPython, but not in CPython.
As well as marking the test with a TODO comment.

Quick Import (recommended):
    python ./scripts/fix_test.py --quick-import cpython/Lib/test/test_foo.py

    This will:
    1. Copy cpython/Lib/test/test_foo.py to Lib/test/test_foo.py (if not exists)
    2. Run the test with RustPython
    3. Mark failing tests with @unittest.expectedFailure

Manual workflow:
1. Copy a specific test from the CPython repository to the RustPython repository.
2. Remove all unexpected failures from the test and skip the tests that hang.
3. Build RustPython: cargo build --release
4. Run from the project root:
   - For single-file tests: python ./scripts/fix_test.py --path ./Lib/test/test_venv.py
   - For package tests: python ./scripts/fix_test.py --path ./Lib/test/test_inspect/test_inspect.py
5. Verify: cargo run --release -- -m test test_venv (should pass with expected failures)
6. Actually fix the tests marked with # TODO: RUSTPYTHON
"""

import argparse
import ast
import re
import shutil
import sys
from pathlib import Path

from lib_updater import (
    COMMENT,
    PatchSpec,
    UtMethod,
    apply_patches,
)


def parse_args():
    parser = argparse.ArgumentParser(description="Fix test.")
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--path", type=Path, help="Path to test file")
    group.add_argument(
        "--quick-import",
        type=Path,
        metavar="PATH",
        help="Import from path containing /Lib/ (e.g., cpython/Lib/test/foo.py)",
    )
    parser.add_argument("--force", action="store_true", help="Force modification")
    parser.add_argument(
        "--platform", action="store_true", help="Platform specific failure"
    )

    args = parser.parse_args()
    return args


class Test:
    name: str = ""
    path: str = ""
    result: str = ""

    def __str__(self):
        return f"Test(name={self.name}, path={self.path}, result={self.result})"


class TestResult:
    tests_result: str = ""
    tests = []
    unexpected_successes = []  # Tests that passed but were marked as expectedFailure
    stdout = ""

    def __str__(self):
        return f"TestResult(tests_result={self.tests_result},tests={len(self.tests)},unexpected_successes={len(self.unexpected_successes)})"


def parse_results(result):
    lines = result.stdout.splitlines()
    test_results = TestResult()
    test_results.tests = []
    test_results.unexpected_successes = []
    test_results.stdout = result.stdout
    in_test_results = False
    for line in lines:
        if re.match(r"Run tests? sequentially", line):
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
    return test_results


def path_to_test(path) -> list[str]:
    # path format: test.module_name[.submodule].ClassName.test_method
    # We need [ClassName, test_method] - always the last 2 elements
    parts = path.split(".")
    return parts[-2:]  # Get class name and method name


def is_super_call_only(func_node: ast.FunctionDef | ast.AsyncFunctionDef) -> bool:
    """Check if the method body is just 'return super().method_name()'."""
    if len(func_node.body) != 1:
        return False
    stmt = func_node.body[0]
    if not isinstance(stmt, ast.Return) or stmt.value is None:
        return False
    # Check for super().method_name() pattern
    call = stmt.value
    if not isinstance(call, ast.Call):
        return False
    if not isinstance(call.func, ast.Attribute):
        return False
    super_call = call.func.value
    if not isinstance(super_call, ast.Call):
        return False
    if not isinstance(super_call.func, ast.Name) or super_call.func.id != "super":
        return False
    return True


def remove_expected_failures(
    contents: str, tests_to_remove: set[tuple[str, str]]
) -> str:
    """Remove @unittest.expectedFailure decorators from tests that now pass."""
    if not tests_to_remove:
        return contents

    tree = ast.parse(contents)
    lines = contents.splitlines()
    lines_to_remove = set()

    for node in ast.walk(tree):
        if not isinstance(node, ast.ClassDef):
            continue
        class_name = node.name
        for item in node.body:
            if not isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)):
                continue
            method_name = item.name
            if (class_name, method_name) not in tests_to_remove:
                continue

            # Check if we should remove the entire method (super() call only)
            remove_entire_method = is_super_call_only(item)

            if remove_entire_method:
                # Remove entire method including decorators and any preceding comment
                first_line = item.lineno - 1  # 0-indexed, def line
                if item.decorator_list:
                    first_line = item.decorator_list[0].lineno - 1
                # Check for TODO comment before first decorator/def
                if first_line > 0:
                    prev_line = lines[first_line - 1].strip()
                    if prev_line.startswith("#") and COMMENT in prev_line:
                        first_line -= 1
                # Remove from first_line to end_lineno (inclusive)
                for i in range(first_line, item.end_lineno):
                    lines_to_remove.add(i)
            else:
                # Only remove the expectedFailure decorator
                for dec in item.decorator_list:
                    dec_line = dec.lineno - 1  # 0-indexed
                    line_content = lines[dec_line]

                    # Check if it's @unittest.expectedFailure
                    if "expectedFailure" not in line_content:
                        continue

                    # Check if TODO: RUSTPYTHON is on the same line or the line before
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

    # Remove lines in reverse order to maintain line numbers
    for line_idx in sorted(lines_to_remove, reverse=True):
        del lines[line_idx]

    return "\n".join(lines) + "\n" if lines else ""


def build_patches(test_parts_set: set[tuple[str, str]]) -> dict:
    """Convert failing tests to lib_updater patch format."""
    patches = {}
    for class_name, method_name in test_parts_set:
        if class_name not in patches:
            patches[class_name] = {}
        patches[class_name][method_name] = [
            PatchSpec(UtMethod.ExpectedFailure, None, "")
        ]
    return patches


def run_test(test_name):
    print(f"Running test: {test_name}")
    rustpython_location = "./target/release/rustpython"
    if sys.platform == "win32":
        rustpython_location += ".exe"

    import subprocess

    result = subprocess.run(
        [rustpython_location, "-m", "test", "-v", "-u", "all", "--slowest", test_name],
        capture_output=True,
        text=True,
    )
    return parse_results(result)


if __name__ == "__main__":
    args = parse_args()

    # Handle --quick-import: extract Lib/... path and copy if needed
    if args.quick_import is not None:
        # Normalize path separators to forward slashes for cross-platform support
        src_str = str(args.quick_import).replace("\\", "/")
        lib_marker = "/Lib/"

        if lib_marker not in src_str:
            print(
                f"Error: --quick-import path must contain '/Lib/' or '\\Lib\\' (got: {args.quick_import})"
            )
            sys.exit(1)

        idx = src_str.index(lib_marker)
        lib_path = Path(src_str[idx + 1 :])  # Lib/test/foo.py
        src_path = args.quick_import

        if not src_path.exists():
            print(f"Error: Source file not found: {src_path}")
            sys.exit(1)

        if not lib_path.exists():
            print(f"Copying: {src_path} -> {lib_path}")
            lib_path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(src_path, lib_path)
        else:
            print(f"File already exists: {lib_path}")

        args.path = lib_path

    test_path = args.path.resolve()
    if not test_path.exists():
        print(f"Error: File not found: {test_path}")
        sys.exit(1)
    # Detect package tests (e.g., test_ctypes/test_random_things.py)
    if test_path.parent.name.startswith("test_"):
        test_name = f"{test_path.parent.name}.{test_path.stem}"
    else:
        test_name = test_path.stem
    tests = run_test(test_name)
    f = test_path.read_text(encoding="utf-8")

    # Collect failing tests (with deduplication for subtests)
    failing_tests = set()  # Track (class_name, method_name) to avoid duplicates
    for test in tests.tests:
        if test.result == "fail" or test.result == "error":
            test_parts = path_to_test(test.path)
            if len(test_parts) == 2:
                test_key = tuple(test_parts)
                if test_key not in failing_tests:
                    failing_tests.add(test_key)
                    print(f"Marking as failing: {test_parts[0]}.{test_parts[1]}")

    # Collect unexpected successes (tests that now pass but have expectedFailure)
    unexpected_successes = set()
    for test in tests.unexpected_successes:
        test_parts = path_to_test(test.path)
        if len(test_parts) == 2:
            test_key = tuple(test_parts)
            if test_key not in unexpected_successes:
                unexpected_successes.add(test_key)
                print(f"Removing expectedFailure: {test_parts[0]}.{test_parts[1]}")

    # Remove expectedFailure from tests that now pass
    if unexpected_successes:
        f = remove_expected_failures(f, unexpected_successes)

    # Apply patches for failing tests
    if failing_tests:
        patches = build_patches(failing_tests)
        f = apply_patches(f, patches)

    # Write changes if any modifications were made
    if failing_tests or unexpected_successes:
        test_path.write_text(f, encoding="utf-8")

    print(f"Added expectedFailure to {len(failing_tests)} tests")
    print(f"Removed expectedFailure from {len(unexpected_successes)} tests")
