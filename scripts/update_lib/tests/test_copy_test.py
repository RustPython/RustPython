"""Tests for bounded, hash-pinned canonical test copying."""

import ast
import contextlib
import hashlib
import io
import tempfile
import unittest
from pathlib import Path

from update_lib.cmd_copy_test import copy_test, main


class TestCopyTest(unittest.TestCase):
    destination = '''# Destination-only header
class Finder:
    if int.__doc__:
        def example():
            r"""Count is below 800. π\\n"""

    def untouched(self):
        return "destination"
'''
    source = '''# Canonical header must not be copied
class Finder:
    if int.__doc__:
        def example():
            r"""Count is below 850. π\\n"""

    def untouched(self):
        return "canonical"
'''

    def test_complete_definition_and_surroundings(self):
        result = copy_test(self.source, self.destination, "Finder.example")
        self.assertEqual(result, self.destination.replace("below 800", "below 850"))
        self.assertIn('return "destination"', result)
        source_method = ast.parse(self.source).body[0].body[0].body[0]
        result_method = ast.parse(result).body[0].body[0].body[0]
        self.assertEqual(
            ast.get_source_segment(self.source, source_method),
            ast.get_source_segment(result, result_method),
        )

    def test_guard_mismatch_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "guards differ"):
            copy_test(
                self.source.replace("if int.__doc__", "if enabled"),
                self.destination,
                "Finder.example",
            )

    def test_missing_or_ambiguous_definition_is_rejected(self):
        for source in ("class Other: pass\n", self.source + self.source):
            with self.subTest(source=source):
                with self.assertRaisesRegex(ValueError, "Expected one class"):
                    copy_test(source, self.destination, "Finder.example")
        duplicate = self.source.replace(
            "    def untouched(self):",
            "    def example():\n        pass\n\n    def untouched(self):",
        )
        with self.assertRaisesRegex(ValueError, "Expected one method"):
            copy_test(duplicate, self.destination, "Finder.example")

    def test_decorator_and_signature_changes_are_rejected(self):
        decorated = self.source.replace(
            "        def example():", "        @decorator\n        def example():"
        )
        with self.assertRaisesRegex(ValueError, "decorators differ"):
            copy_test(decorated, self.destination, "Finder.example")
        with self.assertRaisesRegex(ValueError, "signatures differ"):
            copy_test(
                self.source.replace("def example():", "def example(arg):"),
                self.destination,
                "Finder.example",
            )

    def test_nested_local_scope_is_not_selected(self):
        local = "def factory():\n" + "".join(
            "    " + line for line in self.source.splitlines(True)
        )
        with self.assertRaisesRegex(ValueError, "Expected one class"):
            copy_test(local, self.destination, "Finder.example")

    def test_return_annotation_and_type_parameters_are_rejected(self):
        for declaration in ("def example() -> int:", "def example[T]():"):
            with self.subTest(declaration=declaration):
                source = self.source.replace("def example():", declaration)
                with self.assertRaisesRegex(ValueError, "signatures differ"):
                    copy_test(source, self.destination, "Finder.example")

    def test_cli_hashes_and_no_write_on_stale_input(self):
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "source.py"
            destination = Path(temp) / "destination.py"
            source.write_bytes(self.source.encode())
            destination.write_bytes(self.destination.encode())
            args = [
                str(source),
                str(destination),
                "--test",
                "Finder.example",
                "--source-sha256",
                hashlib.sha256(source.read_bytes()).hexdigest(),
                "--destination-sha256",
                "0" * 64,
            ]
            with (
                contextlib.redirect_stderr(io.StringIO()),
                self.assertRaises(SystemExit),
            ):
                main(args)
            self.assertEqual(destination.read_bytes(), self.destination.encode())
            args[-1] = hashlib.sha256(destination.read_bytes()).hexdigest()
            with contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertEqual(main(args), 0)
            self.assertIn('"source_sha256"', output.getvalue())
            self.assertEqual(
                destination.read_text(),
                self.destination.replace("below 800", "below 850"),
            )
