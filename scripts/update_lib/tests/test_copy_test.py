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

    def test_decorator_comments_must_match_exactly(self):
        for source_marker, destination_marker in (
            (
                "@unittest.expectedFailure",
                "@unittest.expectedFailure  # TODO: RUSTPYTHON; reason",
            ),
            (
                "@unittest.expectedFailure  # original reason",
                "@unittest.expectedFailure  # changed reason",
            ),
            (
                "@unittest.expectedFailure\n        @other",
                "@unittest.expectedFailure\n        # marker rationale\n        @other",
            ),
            (
                "@unittest.expectedFailureIf(\n            condition,\n            'reason',\n        )",
                "@unittest.expectedFailureIf(\n            condition,  # project limitation\n            'reason',\n        )",
            ),
        ):
            with self.subTest(destination_marker=destination_marker):
                source = self.source.replace(
                    "        def example():",
                    f"        {source_marker}\n        def example():",
                )
                destination = self.destination.replace(
                    "        def example():",
                    f"        {destination_marker}\n        def example():",
                )
                with self.assertRaisesRegex(ValueError, "decorator text differs"):
                    copy_test(source, destination, "Finder.example")

    def test_identical_decorator_comments_are_preserved(self):
        prefix = (
            "        @unittest.expectedFailure  # TODO: RUSTPYTHON; reason\n"
            "        # Keep this rationale with the marker.\n"
            "        def example():"
        )
        for declaration in ("def example():", "async def example():"):
            with self.subTest(declaration=declaration):
                source = self.source.replace("        def example():", prefix)
                destination = self.destination.replace("        def example():", prefix)
                source = source.replace("def example():", declaration)
                destination = destination.replace("def example():", declaration)
                self.assertEqual(
                    copy_test(source, destination, "Finder.example"),
                    destination.replace("below 800", "below 850"),
                )

    def test_cli_does_not_write_on_decorator_comment_mismatch(self):
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "source.py"
            destination = Path(temp) / "destination.py"
            source.write_text(
                self.source.replace(
                    "        def example():",
                    "        @unittest.expectedFailure\n        def example():",
                )
            )
            destination.write_text(
                self.destination.replace(
                    "        def example():",
                    "        @unittest.expectedFailure  # TODO: RUSTPYTHON; reason\n"
                    "        def example():",
                )
            )
            before = destination.read_bytes()
            with (
                contextlib.redirect_stderr(io.StringIO()) as errors,
                self.assertRaises(SystemExit),
            ):
                main(
                    [
                        str(source),
                        str(destination),
                        "--test",
                        "Finder.example",
                        "--source-sha256",
                        hashlib.sha256(source.read_bytes()).hexdigest(),
                        "--destination-sha256",
                        hashlib.sha256(before).hexdigest(),
                    ]
                )
            self.assertIn("decorator text differs", errors.getvalue())
            self.assertEqual(destination.read_bytes(), before)

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
