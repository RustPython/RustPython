"""Host-side contract tests for the actual non-JS, no-OS WASM interpreter.

Run with CPython. Python source in these tests executes only in the WASM guest.
This does not provide the OS capabilities needed by CPython's regrtest worker.
"""

import argparse
import json
import os
import subprocess
import tempfile
import time
import unittest
from pathlib import Path


class RuntimeTests(unittest.TestCase):
    runtime: Path
    module: Path

    def run_guest(self, *args, module=None, exit_code=0, status="ok"):
        proc = subprocess.run(
            [str(self.runtime), str(module or self.module), *args],
            capture_output=True,
            encoding="utf-8",
            timeout=90,
        )
        self.assertEqual(proc.returncode, exit_code, (proc.stdout, proc.stderr))
        # A whole-stream decode also rejects extra prints or duplicate records.
        result = json.loads(proc.stdout)
        self.assertEqual(set(result), {"status", "output"})
        self.assertEqual(result["status"], status, result)
        self.assertIsInstance(result["output"], str)
        return result["output"]

    def test_default_expression(self):
        self.assertEqual(self.run_guest(), "4")

    def test_supplied_source_and_no_os(self):
        source = """
import sys
assert sys.platform == 'unknown', sys.platform
assert sys.maxsize == 2**31 - 1, sys.maxsize
assert 'posix' not in sys.builtin_module_names
assert 'nt' not in sys.builtin_module_names
assert 'js' not in sys.builtin_module_names
values = [i * i for i in range(6)]
assert values == [0, 1, 4, 9, 16, 25]
assert (1 << 100).bit_length() == 101
try:
    1 / 0
except ZeroDivisionError:
    pass
else:
    assert False
class Value:
    def __init__(self, value):
        self.value = value
assert Value('한글').value == '한글'
f'guest:{sum(values)}:한글'
"""
        self.assertEqual(self.run_guest("-c", source), "'guest:55:한글'")

    def test_file_source(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source with spaces.py"
            source.write_text("assert 6 * 7 == 42\n'from file: 한글'", encoding="utf-8")
            self.assertEqual(self.run_guest(str(source)), "'from file: 한글'")

    def test_empty_source(self):
        self.assertEqual(self.run_guest("-c", ""), "None")

    def test_assertion_failure(self):
        output = self.run_guest(
            "-c",
            "assert False, 'deliberate failure'",
            exit_code=1,
            status="python_error",
        )
        self.assertIn("AssertionError: deliberate failure", output)

    def test_syntax_error(self):
        output = self.run_guest("-c", "if :", exit_code=1, status="python_error")
        self.assertIn("SyntaxError", output)

    def test_python_timeout(self):
        self.run_guest(
            "-c", "while True: pass", "--timeout", "10", exit_code=124, status="timeout"
        )

    def test_repr_failure(self):
        output = self.run_guest(
            "-c",
            "class C:\n    def __repr__(self):\n        raise ValueError('repr failed')\nC()",
            exit_code=1,
            status="python_error",
        )
        self.assertIn("ValueError: repr failed", output)

    def test_repr_with_surrogate_is_python_error(self):
        output = self.run_guest(
            "-c",
            "class C:\n    def __repr__(self):\n        return chr(0xd800)\nC()",
            exit_code=1,
            status="python_error",
        )
        self.assertIn("UnicodeEncodeError", output)

    def test_invalid_utf8(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "invalid.py"
            source.write_bytes(b"\xff")
            output = self.run_guest(str(source), exit_code=2, status="input_error")
            self.assertIn("UTF-8", output)

    def test_invalid_arguments(self):
        for args in (("-c",), ("--unknown",), ("-c", "1", "-c", "2")):
            with self.subTest(args=args):
                self.run_guest(*args, exit_code=2, status="input_error")
        for timeout in ("0", "-1", "NaN", "inf", "nonsense"):
            with self.subTest(timeout=timeout):
                self.run_guest("--timeout", timeout, exit_code=2, status="input_error")

    def test_unreadable_source(self):
        with tempfile.TemporaryDirectory() as directory:
            self.run_guest(
                str(Path(directory) / "missing.py"),
                exit_code=2,
                status="input_error",
            )

    @unittest.skipUnless(hasattr(os, "mkfifo"), "requires named pipes")
    def test_blocked_source_read_times_out(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "blocked.py"
            os.mkfifo(source)
            self.run_guest(
                str(source), "--timeout", "0.05", exit_code=124, status="timeout"
            )

    def test_source_size_limit(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "large.py"
            source.write_bytes(b" " * (16 * 1024 * 1024 + 1))
            output = self.run_guest(str(source), exit_code=2, status="input_error")
            self.assertIn("source exceeds", output)

    def run_module(self, body, *args, **kwargs):
        # Wasmer accepts WAT, so test malformed ABI and traps without another
        # guest compiler, a JS runtime, or an operating-system shim.
        with tempfile.TemporaryDirectory() as directory:
            module = Path(directory) / "contract.wat"
            module.write_text(body, encoding="utf-8")
            return self.run_guest(*args, module=module, **kwargs)

    @staticmethod
    def module_with_eval(body, imports="", *, alloc="i32.const 0", free=""):
        return f"""(module
            {imports}
            (memory (export "memory") 1)
            (func (export "alloc_source") (param i32) (result i32) {alloc})
            (func (export "free_source") (param i32 i32) {free})
            (func (export "eval") (param i32 i32) (result i32) {body})
        )"""

    def test_trap_is_not_success(self):
        output = self.run_module(
            self.module_with_eval("unreachable"), exit_code=2, status="runtime_error"
        )
        self.assertIn("unreachable", output.lower())

    def test_source_allocation_errors(self):
        for alloc in ("unreachable", "i32.const 65535"):
            with self.subTest(alloc=alloc):
                output = self.run_module(
                    self.module_with_eval("i32.const 0", alloc=alloc),
                    exit_code=2,
                    status="runtime_error",
                )
                expected = "unreachable" if alloc == "unreachable" else "out of bounds"
                self.assertIn(expected, output.lower())

    def test_source_deallocation_trap(self):
        output = self.run_module(
            self.module_with_eval("i32.const 0", free="unreachable"),
            exit_code=2,
            status="runtime_error",
        )
        self.assertIn("unreachable", output.lower())

    def test_timeout_is_not_success(self):
        started = time.monotonic()
        self.run_module(
            self.module_with_eval("(loop $forever (br $forever)) i32.const 0"),
            "--timeout",
            "1",
            exit_code=124,
            status="timeout",
        )
        self.assertGreaterEqual(time.monotonic() - started, 0.9)

    def test_unknown_guest_status(self):
        output = self.run_module(
            self.module_with_eval("i32.const 99"), exit_code=2, status="runtime_error"
        )
        self.assertIn("unknown guest status: 99", output)

    def test_missing_result(self):
        output = self.run_module(
            self.module_with_eval("i32.const 0"), exit_code=2, status="runtime_error"
        )
        self.assertIn("guest did not report a result", output)

    def test_duplicate_result(self):
        output = self.run_module(
            self.module_with_eval(
                "i32.const 0 i32.const 0 call $print drop "
                "i32.const 0 i32.const 0 call $print",
                '(import "env" "print" (func $print (param i32 i32) (result i32)))',
            ),
            exit_code=2,
            status="runtime_error",
        )
        self.assertIn("guest reported more than one result", output)

    def test_invalid_utf8_result(self):
        output = self.run_module(
            self.module_with_eval(
                "i32.const 0 i32.const 255 i32.store8 "
                "i32.const 0 i32.const 1 call $print",
                '(import "env" "print" (func $print (param i32 i32) (result i32)))',
            ),
            exit_code=2,
            status="runtime_error",
        )
        self.assertIn("utf-8", output)

    def test_missing_export(self):
        self.run_module("(module)", exit_code=2, status="runtime_error")

    def test_unexpected_import(self):
        for name in ("wasi_snapshot_preview1", "wbindgen", "env"):
            with self.subTest(name=name):
                output = self.run_module(
                    f'(module (import "{name}" "unexpected" (func)))',
                    exit_code=2,
                    status="runtime_error",
                )
                self.assertIn(f"unexpected import: {name}.unexpected", output)

    def test_output_outside_memory(self):
        output = self.run_module(
            self.module_with_eval(
                "i32.const 65535 i32.const 2 call $print",
                '(import "env" "print" (func $print (param i32 i32) (result i32)))',
            ),
            exit_code=2,
            status="runtime_error",
        )
        self.assertIn("output is outside guest memory", output)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--module", type=Path, required=True)
    options, unittest_args = parser.parse_known_args()
    RuntimeTests.runtime = options.runtime.resolve(strict=True)
    RuntimeTests.module = options.module.resolve(strict=True)
    unittest.main(argv=[__file__, *unittest_args])
