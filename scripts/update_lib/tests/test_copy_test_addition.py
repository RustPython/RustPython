"""Metadata-only coverage of absent-only canonical method insertion."""

import contextlib
import hashlib
import io
import json
import tempfile
import unittest
from pathlib import Path

from update_lib.cmd_copy_test import main
from update_lib.copy_test_addition import add_test


class TestAddTest(unittest.TestCase):
    prefix = '''class Cases(unittest.TestCase):
    """Preserve the existing class."""

    def anchor(self):
        self.assertEqual(1, 1)
'''
    addition = """
    # Canonical method comment.
    def test_new(self):
        value = len("π")
        self.assertEqual(value, 1)
"""
    suffix = """

# Destination-only marker must remain byte-identical.
@unittest.skip("TODO: RUSTPYTHON; unrelated")
class Other(unittest.TestCase):
    pass
"""
    destination = "import unittest\n# Local header\n" + prefix + suffix
    source = "import unittest\n# Upstream header\n" + prefix + addition

    def add(self, source=None, destination=None, **kwargs):
        return add_test(
            self.source if source is None else source,
            self.destination if destination is None else destination,
            "Cases.test_new",
            "Cases.anchor",
            **kwargs,
        )

    def test_exact_insertion_and_preserved_surroundings(self):
        result = self.add()
        self.assertEqual(
            result, self.destination.replace(self.prefix, self.prefix + self.addition)
        )
        self.assertEqual(result.replace(self.addition, "", 1), self.destination)

    def test_sequential_additions_and_repeat_rejection(self):
        second = "\n    def test_next(self):\n        pass\n"
        source = self.source + second
        first = self.add(source)
        result = add_test(source, first, "Cases.test_next", "Cases.test_new")
        self.assertEqual(result, first.replace(self.addition, self.addition + second))
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.add(source, first)

    def test_crlf_and_raw_string_bytes_are_preserved(self):
        addition = self.addition.replace('len("π")', 'len(r"π\\n")')
        source = (self.source.replace(self.addition, addition)).replace("\n", "\r\n")
        destination = self.destination.replace("\n", "\r\n")
        result = self.add(source, destination)
        self.assertEqual(
            result.replace(addition.replace("\n", "\r\n"), "", 1), destination
        )

    def test_nonstandard_line_separators_are_rejected(self):
        for separator in "\v\f\x1c\x1d\x1e\x85\u2028\u2029":
            with self.subTest(separator=repr(separator)):
                with self.assertRaisesRegex(ValueError, "Nonstandard line separators"):
                    self.add(self.source.replace("π", "π" + separator))
                with self.assertRaisesRegex(ValueError, "Nonstandard line separators"):
                    self.add(destination=self.destination + "# " + separator)

    def test_class_and_method_ambiguities_are_rejected(self):
        for source in (
            self.source + self.prefix,
            self.source + "Cases = object\n",
            self.source + "Alias = Cases\n",
            self.source + self.addition,
            self.source.replace("    def anchor", "    anchor = 1\n    def anchor"),
            self.source.replace("    def anchor", "    if True:\n        def anchor"),
            "if True:\n"
            + "".join("    " + line for line in self.source.splitlines(True)),
            self.source.replace("    def anchor", "    async def anchor"),
        ):
            with (
                self.subTest(source=source),
                self.assertRaises((SyntaxError, ValueError)),
            ):
                self.add(source)

    def test_anchor_and_class_prefix_must_match(self):
        for destination in (
            self.destination.replace("anchor", "other_anchor"),
            self.destination.replace(
                "self.assertEqual(1, 1)", "self.assertEqual(2, 2)"
            ),
            self.destination.replace(
                "    def anchor",
                "    @unittest.expectedFailure  # marker\n    def anchor",
            ),
            self.destination.replace(
                self.prefix, self.prefix + "\n    def another(self):\n        pass\n"
            ),
            self.destination.replace("unittest.TestCase", "object", 1),
        ):
            with self.subTest(destination=destination), self.assertRaises(ValueError):
                self.add(destination=destination)
        for name, anchor in (
            ("Cases.test_new", "Other.anchor"),
            ("Cases.anchor", "Cases.anchor"),
            ("bad", "Cases.anchor"),
        ):
            with self.subTest(name=name, anchor=anchor), self.assertRaises(ValueError):
                add_test(self.source, self.destination, name, anchor)

    def test_unsupported_class_headers_and_boundaries(self):
        for replacement in (
            "@decorator\nclass Cases(unittest.TestCase):",
            "class Cases(unittest.TestCase, metaclass=Meta):",
            "class Cases[T](unittest.TestCase):",
            "class Cases(factory()):",
            'class Cases(unittest.TestCase): "inline doc";',
        ):
            with (
                self.subTest(replacement=replacement),
                self.assertRaises((SyntaxError, ValueError)),
            ):
                self.add(
                    self.source.replace("class Cases(unittest.TestCase):", replacement)
                )
        with self.assertRaisesRegex(ValueError, "Ambiguous comments"):
            self.add(self.source.replace("    # Canonical", "# Canonical"))
        with self.assertRaisesRegex(ValueError, "complete source line"):
            self.add(self.source.rstrip("\n"))

    def test_selected_decorators_and_signatures_are_rejected(self):
        for declaration in (
            "@unittest.expectedFailure\n    def test_new(self):",
            "def test_new(self, arg=1):",
            "def test_new(self: object):",
            "def test_new(self) -> None:",
            "def test_new[T](self):",
            "async def test_new(self):",
        ):
            with self.subTest(declaration=declaration), self.assertRaises(ValueError):
                self.add(self.source.replace("def test_new(self):", declaration))

    def test_import_dependencies_and_builtin_shadowing(self):
        for source, destination in (
            (self.source.replace("len(", "helper("), self.destination),
            (
                "def helper(x): return x\n" + self.source.replace("len(", "helper("),
                "def helper(x): return x\n" + self.destination,
            ),
            ("len = object\n" + self.source, self.destination),
            (self.source, "len = object\n" + self.destination),
            ("from package import *\n" + self.source, self.destination),
            (
                self.source,
                self.destination.replace("import unittest", "import other as unittest"),
            ),
            (self.source, self.destination + "import unittest\n"),
            (self.source, self.destination + "unittest.TestCase.extra = 1\n"),
            (self.source, "from __future__ import annotations\n" + self.destination),
        ):
            with (
                self.subTest(source=source, destination=destination),
                self.assertRaises(ValueError),
            ):
                self.add(source, destination)
        source = "import math\n" + self.source.replace('len("π")', "math.floor(1)")
        self.add(source, "import math\n" + self.destination)

    def test_preserved_method_headers_and_bodies_are_checked(self):
        for declaration in (
            "def anchor(self, arg=missing):",
            "def anchor(self, arg: missing):",
            "def anchor(self) -> missing:",
            "@missing\n    def anchor(self):",
            "def anchor(self, arg=(lambda: 1)):",
        ):
            source = self.source.replace("def anchor(self):", declaration)
            destination = self.destination.replace("def anchor(self):", declaration)
            with self.subTest(declaration=declaration), self.assertRaises(ValueError):
                self.add(source, destination)
        for body in (
            "self.test_new()",
            "self.test_later()",
            "getattr(self, 'test_new')()",
            "global len\n        len = 1",
            "class Nested: pass",
        ):
            source = self.source.replace("self.assertEqual(1, 1)", body)
            destination = self.destination.replace("self.assertEqual(1, 1)", body)
            if "test_later" in body:
                source += "\n    def test_later(self):\n        pass\n"
            with self.subTest(body=body), self.assertRaises(ValueError):
                self.add(source, destination)

    def test_local_import_requires_exact_acknowledgment(self):
        source = self.source.replace(
            'value = len("π")', "import absent_module\n        value = 1"
        )
        with self.assertRaisesRegex(ValueError, "exact explicit allowlist"):
            self.add(source)
        self.add(source, local_imports=["import absent_module"])
        for allowed in (
            ["import other"],
            ["import absent_module", "import extra"],
            ["import absent_module", "import absent_module"],
            ["pass"],
            ["import absent_module; import extra"],
            ["from . import absent_module"],
            ["from absent_module import *"],
        ):
            with self.subTest(allowed=allowed), self.assertRaises(ValueError):
                self.add(source, local_imports=allowed)
        for statement in (
            "import absent_module\n        import absent_module",
            "import absent_module, absent_module",
            "import absent_module as self",
            "import absent_module\n        absent_module = 1",
        ):
            changed = source.replace("import absent_module", statement)
            with (
                self.subTest(statement=statement),
                self.assertRaises((SyntaxError, ValueError)),
            ):
                self.add(changed, local_imports=[statement])

    def test_local_import_binding_forms_are_rejected(self):
        for declaration in (
            "def absent_module(): pass",
            "async def absent_module(): pass",
            "class absent_module: pass",
            "try:\n            pass\n        except Exception as absent_module:\n            pass",
            "match 1:\n            case absent_module:\n                pass",
            "match []:\n            case [*absent_module]:\n                pass",
            "match {}:\n            case {**absent_module}:\n                pass",
            "global absent_module",
            "nonlocal absent_module",
        ):
            source = self.source.replace(
                'value = len("π")',
                "import absent_module\n        " + declaration + "\n        value = 1",
            )
            with self.subTest(declaration=declaration):
                with self.assertRaisesRegex(
                    ValueError, "cannot shadow arguments or be rebound"
                ):
                    self.add(source, local_imports=["import absent_module"])

    def test_cli_receipt_and_no_write_on_repeat_or_stale_hash(self):
        with tempfile.TemporaryDirectory() as temp:
            source, destination = (
                Path(temp) / "source.py",
                Path(temp) / "destination.py",
            )
            source.write_bytes(self.source.encode())
            destination.write_bytes(self.destination.encode())
            args = [
                str(source),
                str(destination),
                "--test",
                "Cases.test_new",
                "--add-after",
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
            self.assertEqual(destination.read_bytes(), self.destination.encode())
            args[-1] = hashlib.sha256(destination.read_bytes()).hexdigest()
            with contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertEqual(main(args), 0)
            receipt = json.loads(output.getvalue())
            self.assertEqual(receipt["mode"], "add")
            self.assertEqual(receipt["anchor"], "Cases.anchor")
            self.assertEqual(receipt["local_imports"], [])
            self.assertEqual(destination.read_text(), self.add())
            before = destination.read_bytes()
            args[-1] = hashlib.sha256(before).hexdigest()
            with (
                contextlib.redirect_stderr(io.StringIO()),
                self.assertRaises(SystemExit),
            ):
                main(args)
            self.assertEqual(destination.read_bytes(), before)
