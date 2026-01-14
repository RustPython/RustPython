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
import shutil
import sys
from pathlib import Path

from lib_updater import PatchSpec, UtMethod, apply_patches


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
    seen_tests = set()  # Track (class_name, method_name) to avoid duplicates
    for test in tests.tests:
        if test.result == "fail" or test.result == "error":
            test_parts = path_to_test(test.path)
            if len(test_parts) == 2:
                test_key = tuple(test_parts)
                if test_key not in seen_tests:
                    seen_tests.add(test_key)
                    print(f"Marking test: {test_parts[0]}.{test_parts[1]}")

    # Apply patches using lib_updater
    if seen_tests:
        patches = build_patches(seen_tests)
        f = apply_patches(f, patches)
        test_path.write_text(f, encoding="utf-8")

    print(f"Modified {len(seen_tests)} tests")
