"""
An automated script to mark failures in python test suite.
It adds @unittest.expectedFailure to the test functions that are failing in RustPython, but not in CPython.
As well as marking the test with a TODO comment.

How to use:
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
import itertools
import platform
import sys
from pathlib import Path


def parse_args():
    parser = argparse.ArgumentParser(description="Fix test.")
    parser.add_argument("--path", type=Path, help="Path to test file")
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
    stdout = ""

    def __str__(self):
        return f"TestResult(tests_result={self.tests_result},tests={len(self.tests)})"


def parse_results(result):
    lines = result.stdout.splitlines()
    test_results = TestResult()
    test_results.stdout = result.stdout
    in_test_results = False
    for line in lines:
        if line == "Run tests sequentially":
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
    return test_results


def path_to_test(path) -> list[str]:
    # path format: test.module_name[.submodule].ClassName.test_method
    # We need [ClassName, test_method] - always the last 2 elements
    parts = path.split(".")
    return parts[-2:]  # Get class name and method name


def find_test_lineno(file: str, test: list[str]) -> tuple[int, int] | None:
    """Find the line number and column offset of a test function.
    Returns (lineno, col_offset) or None if not found.
    """
    a = ast.parse(file)
    for key, node in ast.iter_fields(a):
        if key == "body":
            for n in node:
                match n:
                    case ast.ClassDef():
                        if len(test) == 2 and test[0] == n.name:
                            for fn in n.body:
                                match fn:
                                    case ast.FunctionDef() | ast.AsyncFunctionDef():
                                        if fn.name == test[-1]:
                                            return (fn.lineno, fn.col_offset)
                    case ast.FunctionDef() | ast.AsyncFunctionDef():
                        if n.name == test[0] and len(test) == 1:
                            return (n.lineno, n.col_offset)
    return None


def apply_modifications(file: str, modifications: list[tuple[int, int]]) -> str:
    """Apply all modifications in reverse order to avoid line number offset issues."""
    lines = file.splitlines()
    fixture = "@unittest.expectedFailure"
    # Sort by line number in descending order
    modifications.sort(key=lambda x: x[0], reverse=True)
    for lineno, col_offset in modifications:
        indent = " " * col_offset
        lines.insert(lineno - 1, indent + fixture)
        lines.insert(lineno - 1, indent + "# TODO: RUSTPYTHON")
    return "\n".join(lines)


def run_test(test_name):
    print(f"Running test: {test_name}")
    rustpython_location = "./target/release/rustpython"
    if sys.platform == "win32":
        rustpython_location += ".exe"

    import subprocess

    result = subprocess.run(
        [rustpython_location, "-m", "test", "-v", test_name],
        capture_output=True,
        text=True,
    )
    return parse_results(result)


if __name__ == "__main__":
    args = parse_args()
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

    # Collect all modifications first (with deduplication for subtests)
    modifications = []
    seen_tests = set()  # Track (class_name, method_name) to avoid duplicates
    for test in tests.tests:
        if test.result == "fail" or test.result == "error":
            test_parts = path_to_test(test.path)
            test_key = tuple(test_parts)
            if test_key in seen_tests:
                continue  # Skip duplicate (same test, different subtest)
            seen_tests.add(test_key)
            location = find_test_lineno(f, test_parts)
            if location:
                print(f"Modifying test: {test.name} at line {location[0]}")
                modifications.append(location)
            else:
                print(f"Warning: Could not find test: {test.name} ({test_parts})")

    # Apply all modifications in reverse order
    if modifications:
        f = apply_modifications(f, modifications)
        test_path.write_text(f, encoding="utf-8")

    print(f"Modified {len(modifications)} tests")
