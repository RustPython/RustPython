"""Tests for bounded, hash-pinned canonical test copying."""

import ast
import contextlib
import hashlib
import io
import tempfile
import unittest
from pathlib import Path

from update_lib.cmd_copy_test import copy_test, insert_test, main


class TestInsertTest(unittest.TestCase):
    source = """# Canonical header
class Cases:
    @marker  # Canonical explanation
    async def added(self):
        return "π"

    def anchor(self):
        return "unchanged"
"""
    destination = """# Preserve this header
class Cases:
    def anchor(self):
        return "unchanged"

    def local(self):
        return "keep this too"
"""

    def insert(self, source=None, destination=None):
        return insert_test(
            self.source if source is None else source,
            self.destination if destination is None else destination,
            "Cases.added",
            "Cases.anchor",
        )

    def test_verbatim_insertion_preserves_surroundings(self):
        block = self.source.split("class Cases:\n", 1)[1].split("    def anchor", 1)[0]
        self.assertEqual(
            self.insert(),
            self.destination.replace("    def anchor", block + "    def anchor"),
        )

    def test_default_copy_still_rejects_absent_method(self):
        with self.assertRaisesRegex(ValueError, "Expected one method"):
            copy_test(self.source, self.destination, "Cases.added")

    def test_unicode_separator_is_not_a_source_line_boundary(self):
        source = self.source.replace('"π"', '"π\u2028value"')
        self.assertEqual(
            self.insert(source=source), self.insert().replace('"π"', '"π\u2028value"')
        )

    def test_duplicate_and_assigned_names_are_rejected(self):
        for binding in (
            "    added = None\n",
            "    def added(self):\n        pass\n",
            "    import module as added\n",
        ):
            with (
                self.subTest(binding=binding),
                self.assertRaisesRegex(ValueError, "already exists"),
            ):
                self.insert(
                    destination=self.destination.replace(
                        "    def anchor", binding + "    def anchor"
                    )
                )

    def test_headers_anchors_and_order_must_match(self):
        cases = (
            (
                self.source,
                self.destination.replace("class Cases:", "class Cases(Base):"),
                "headers differ",
            ),
            (
                self.source,
                self.destination.replace('"unchanged"', '"changed"'),
                "anchor text differs",
            ),
            (
                self.source.replace(
                    "    def anchor", "    unrelated = 1\n\n    def anchor"
                ),
                self.destination,
                "immediately precede",
            ),
        )
        for source, destination, error in cases:
            with self.subTest(error=error), self.assertRaisesRegex(ValueError, error):
                self.insert(source, destination)

    def test_pattern_exception_and_scope_names_are_rejected(self):
        for binding in (
            "    match value:\n        case added:\n            pass\n",
            "    match value:\n        case [*added]:\n            pass\n",
            "    match value:\n        case {**added}:\n            pass\n",
            "    try:\n        pass\n    except Error as added:\n        pass\n",
        ):
            with (
                self.subTest(binding=binding),
                self.assertRaisesRegex(ValueError, "already exists"),
            ):
                self.insert(
                    destination=self.destination.replace(
                        "    def anchor", binding + "    def anchor"
                    )
                )
        for declaration in ("global added", "global anchor", "nonlocal added"):
            for side in ("source", "destination"):
                text = getattr(self, side).replace(
                    "class Cases:\n", f"class Cases:\n    {declaration}\n"
                )
                with (
                    self.subTest(declaration=declaration, side=side),
                    self.assertRaisesRegex(ValueError, "scope declarations"),
                ):
                    self.insert(**{side: text})

    def test_guarded_and_ambiguous_classes_are_rejected(self):
        for source in (
            self.source + self.source,
            "if enabled:\n"
            + "".join("    " + line for line in self.source.splitlines(True)),
        ):
            with self.subTest(source=source), self.assertRaises(ValueError):
                self.insert(source=source)
        with self.assertRaisesRegex(ValueError, "Expected one class"):
            self.insert(destination="class Other:\n    pass\n")
        with self.assertRaisesRegex(ValueError, "same class"):
            insert_test(
                self.source + "class Other:\n    def anchor(self):\n        pass\n",
                self.destination
                + "class Other:\n    def anchor(self):\n        pass\n",
                "Cases.added",
                "Other.anchor",
            )

    def test_comments_at_boundaries_are_not_split_or_discarded(self):
        for source, destination in (
            (
                self.source.replace("    @marker", "    # Test rationale\n    @marker"),
                self.destination,
            ),
            (
                self.source.replace(
                    "    def anchor", "    # Anchor rationale\n    def anchor"
                ),
                self.destination,
            ),
            (
                self.source,
                self.destination.replace(
                    "    def anchor", "    # Local rationale\n    def anchor"
                ),
            ),
        ):
            with (
                self.subTest(source=source, destination=destination),
                self.assertRaises(ValueError),
            ):
                self.insert(source, destination)

    def test_non_lf_and_missing_final_newline_are_rejected(self):
        for text in (self.source.replace("\n", "\r\n"), self.source.rstrip("\n")):
            with (
                self.subTest(text=text),
                self.assertRaisesRegex(ValueError, "LF input"),
            ):
                self.insert(source=text)
        with self.assertRaisesRegex(ValueError, "LF input"):
            self.insert(destination=self.destination.replace("\n", "\r\n"))

    def test_cli_hashes_encoding_and_no_write_on_error(self):
        with tempfile.TemporaryDirectory() as temp:
            source, destination = (
                Path(temp) / "source.py",
                Path(temp) / "destination.py",
            )
            source.write_bytes(self.source.encode("utf-8"))
            before = self.destination.encode("utf-8")
            destination.write_bytes(before)
            args = [
                str(source),
                str(destination),
                "--test",
                "Cases.added",
                "--insert-before",
                "Cases.anchor",
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
            self.assertEqual(destination.read_bytes(), before)
            args[-1] = hashlib.sha256(before).hexdigest()
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(main(args), 0)
            self.assertEqual(destination.read_text(), self.insert())
            destination.write_bytes(before)
            source.write_bytes(b"\xff\n")
            args[-3] = hashlib.sha256(source.read_bytes()).hexdigest()
            with (
                contextlib.redirect_stderr(io.StringIO()),
                self.assertRaises(SystemExit),
            ):
                main(args)
            self.assertEqual(destination.read_bytes(), before)


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
