"""Host-side contract tests for the non-JS, no-OS WASM interpreter."""

import argparse
import subprocess
import tempfile
import unittest
from pathlib import Path


class RuntimeTests(unittest.TestCase):
    runtime: Path
    module: Path

    def run_guest(self, *args, module=None, exit_code=0):
        proc = subprocess.run(
            [str(self.runtime), str(module or self.module), *args],
            capture_output=True,
            encoding="utf-8",
            timeout=120,
        )
        self.assertEqual(proc.returncode, exit_code, (proc.stdout, proc.stderr))
        return proc.stdout

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
        self.assertEqual(self.run_guest("-c", source), "'guest:55:한글'\n")

    def test_host_storage(self):
        source = """
import kv
assert kv.get(b'missing') is None
kv.put(b'k', b'v')
assert kv.get(b'k') == b'v'
kv.put(b'k', b'replaced')
big = bytes(range(256)) * 64
kv.put(b'big', big)
assert kv.get(b'big') == big
(kv.get(b'k'), len(kv.get(b'big')))
"""
        self.assertEqual(self.run_guest("-c", source), "(b'replaced', 16384)\n")

    def test_file_source(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source with spaces.py"
            source.write_text("assert 6 * 7 == 42\n'from file: 한글'", encoding="utf-8")
            self.assertEqual(self.run_guest(str(source)), "'from file: 한글'\n")

    def test_empty_source(self):
        self.assertEqual(self.run_guest("-c", ""), "None\n")

    def test_python_exceptions(self):
        cases = (
            (
                "assert False, 'deliberate failure'",
                "AssertionError: deliberate failure",
            ),
            ("if :", "SyntaxError"),
            (
                "class C:\n    def __repr__(self):\n        raise ValueError('repr failed')\nC()",
                "ValueError: repr failed",
            ),
        )
        for source, needle in cases:
            with self.subTest(source=source):
                self.assertIn(needle, self.run_guest("-c", source, exit_code=1))

    def test_invalid_utf8(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "invalid.py"
            source.write_bytes(b"\xff")
            self.run_guest(str(source), exit_code=2)

    def test_missing_source(self):
        self.run_guest(exit_code=2)

    def test_unexpected_import(self):
        with tempfile.TemporaryDirectory() as directory:
            module = Path(directory) / "contract.wat"
            module.write_text(
                '(module (import "wasi_snapshot_preview1" "fd_write" (func)))',
                encoding="utf-8",
            )
            proc = subprocess.run(
                [str(self.runtime), str(module), "-c", ""],
                capture_output=True,
                encoding="utf-8",
                timeout=120,
            )
            self.assertEqual(proc.returncode, 2, (proc.stdout, proc.stderr))
            self.assertIn("unexpected import", proc.stderr)

    def test_trap(self):
        with tempfile.TemporaryDirectory() as directory:
            module = Path(directory) / "contract.wat"
            module.write_text(
                """(module
                    (memory (export "memory") 1)
                    (func (export "alloc_source") (param i32) (result i32) i32.const 0)
                    (func (export "eval") (param i32 i32) (result i32) unreachable)
                )""",
                encoding="utf-8",
            )
            self.run_guest("-c", "", module=module, exit_code=2)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--module", type=Path, required=True)
    options, unittest_args = parser.parse_known_args()
    RuntimeTests.runtime = options.runtime.resolve(strict=True)
    RuntimeTests.module = options.module.resolve(strict=True)
    unittest.main(argv=[__file__, *unittest_args])
