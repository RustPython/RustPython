"""Synthetic-text coverage of structural canonical method insertion."""

import contextlib
import hashlib
import io
import tempfile
import unittest
from pathlib import Path

from update_lib.cmd_copy_test import copy_test, main
from update_lib.copy_test_insertion import insert_test


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

    def test_cli_modes_are_mutually_exclusive_without_writing(self):
        self.check_cli_rejection(
            ["--add-after", "Cases.anchor"], "not allowed with argument"
        )

    def test_cli_insert_before_cannot_acknowledge_local_imports(self):
        self.check_cli_rejection(
            ["--allow-local-import", "import os"],
            "Local import acknowledgment requires --add-after",
        )

    def check_cli_rejection(self, extra, error):
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
                hashlib.sha256(before).hexdigest(),
                *extra,
            ]
            with (
                contextlib.redirect_stderr(io.StringIO()) as errors,
                self.assertRaises(SystemExit),
            ):
                main(args)
            self.assertIn(error, errors.getvalue())
            self.assertEqual(destination.read_bytes(), before)

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

    def test_unrelated_method_scopes_do_not_block_insertion(self):
        extra = """
    def unrelated(self):
        global added, anchor, AssertionError
        added = None
        wr = None
        e = None
        def inner():
            nonlocal wr, e
            wr = e
        return inner
"""
        self.assertEqual(
            self.insert(self.source + extra, self.destination + extra),
            self.insert() + extra,
        )

    def test_nested_class_lambda_and_comprehension_bindings_are_local(self):
        for extra in (
            "\n    class Nested:\n        global added, anchor\n        added = None\n",
            "\n    callback = lambda: (added := None)\n",
            "\n    items = [added for added in values]\n",
        ):
            with self.subTest(extra=extra):
                self.assertEqual(
                    self.insert(self.source + extra, self.destination + extra),
                    self.insert() + extra,
                )

    def test_definition_header_bindings_still_conflict(self):
        for extra in (
            "\n    def unrelated(arg=(added := 1)):\n        pass\n",
            "\n    def unrelated(arg: (added := int)):\n        pass\n",
            "\n    @decorate(added := 1)\n    def unrelated():\n        pass\n",
            "\n    class Nested((added := Base)):\n        pass\n",
            "\n    callback = lambda arg=(added := 1): arg\n",
        ):
            with (
                self.subTest(extra=extra),
                self.assertRaisesRegex(ValueError, "already exists"),
            ):
                self.insert(destination=self.destination + extra)

    def test_class_control_flow_bindings_still_conflict(self):
        extra = "\n    if enabled:\n        added = None\n"
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.insert(destination=self.destination + extra)

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
