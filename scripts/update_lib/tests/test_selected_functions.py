"""Source-only selective function migration, without importing copied helpers."""

import contextlib
import hashlib
import io
import json
import pathlib
import tempfile
import unittest
from unittest.mock import patch

from update_lib.cmd_migrate import copy_selected_functions, main


class TestSelectedFunctions(unittest.TestCase):
    names = ["available", "requires_available"]
    exports = names
    imports = "import functools\nimport os\nimport sys\nimport unittest\n\n"
    export_line = '    "available", "requires_available",\n'
    dependencies = 'IS_MOBILE = sys.platform == "mobile"\nAVAILABLE = not IS_MOBILE\n\n'
    block = (
        "@functools.cache\n"
        "def available(limit=1):\n"
        "    import errno\n"
        "    try:\n"
        '        return AVAILABLE and bool(errno.EINVAL) and os.name != "" and limit > 0\n'
        "    except ValueError as error:\n"
        "        return bool(str(error))\n\n\n"
        "def requires_available():\n"
        '    return unittest.skipUnless(available(), "needs availability")\n\n\n'
    )
    anchor = "# Keep this comment with STATUS.\nSTATUS = 1\n"
    source = (
        imports
        + '__all__ = [\n    "existing",\n'
        + export_line
        + "]\n\n"
        + dependencies
        + block
        + anchor
    )
    destination = (
        imports
        + '__all__ = [\n    "existing",\n]\n\n'
        + dependencies
        + 'def existing(sys):\n    return "keep π"\n\n'
        + anchor
    )

    def copy(self, source=None, destination=None, names=None, exports=None):
        return copy_selected_functions(
            self.source if source is None else source,
            self.destination if destination is None else destination,
            self.names if names is None else names,
            self.exports if exports is None else exports,
        )

    def test_exact_functions_exports_and_unrelated_bytes(self):
        self.assertEqual(
            self.copy(),
            self.destination.replace(
                "__all__ = [\n", "__all__ = [\n" + self.export_line
            ).replace(self.anchor, self.block + self.anchor),
        )
        self.assertEqual(self.copy().count(self.anchor), 1)

    def test_destination_line_endings_and_local_adaptations_survive(self):
        source = self.source.replace(self.anchor, "STATUS = 1\n")
        destination = self.destination.replace(self.anchor, "STATUS = 1\n").replace(
            "\n", "\r\n"
        )
        self.assertEqual(
            self.copy(source, destination),
            destination.replace(
                "__all__ = [\r\n", "__all__ = [\r\n" + self.export_line
            ).replace("STATUS = 1\r\n", self.block + "STATUS = 1\r\n"),
        )

    def test_local_imports_closures_and_comprehensions_are_not_global_dependencies(
        self,
    ):
        block = (
            "@functools.cache\n"
            "def available(items=()):\n"
            "    def inner(value):\n"
            "        return value in items\n"
            "    return [inner(value) for value in items]\n\n\n"
            "async def requires_available():\n"
            "    import errno as local\n"
            "    return local.EINVAL\n\n\n"
        )
        self.assertIn(block, self.copy(self.source.replace(self.block, block)))
        with self.assertRaises(ValueError):
            self.copy(
                self.source.replace(
                    self.block, block.replace("value in items", "MISSING", 1)
                )
            )

    def test_multiple_decorators_defaults_and_body_are_copied_without_execution(self):
        block = self.block.replace(
            "@functools.cache\ndef available(limit=1):",
            "@functools.cache\n@functools.lru_cache(\n    maxsize=None\n)\n"
            "def available(limit=os.name):",
        ).replace(
            "    import errno\n",
            "    raise RuntimeError('must not run')\n    import errno\n",
        )
        self.assertIn(block, self.copy(self.source.replace(self.block, block)))

    def test_selection_and_existing_bindings_fail_closed(self):
        for source, destination, names in (
            (self.source, self.destination, []),
            (self.source, self.destination, self.names * 2),
            (self.source, self.destination, list(reversed(self.names))),
            (self.source, self.destination, ["missing"]),
            (self.source + "available = None\n", self.destination, self.names),
            (self.source, self.destination + "del available\n", self.names),
            (self.source, self.destination + "import other as available\n", self.names),
            (
                self.source,
                self.destination + "if condition:\n    available = None\n",
                self.names,
            ),
            (
                self.source.replace(
                    "def requires_available", "UNSELECTED = 1\ndef requires_available"
                ),
                self.destination,
                self.names,
            ),
            (self.source.replace(self.anchor, ""), self.destination, self.names),
        ):
            with self.subTest(names=names), self.assertRaises(ValueError):
                self.copy(source, destination, names)

    def test_missing_changed_late_and_rebound_dependencies_fail_closed(self):
        for destination in (
            self.destination.replace("import functools\n", ""),
            self.destination.replace(
                "import functools\n", "import other as functools\n"
            ),
            self.destination.replace("import os\n", "") + "import os\n",
            self.destination.replace(
                'sys.platform == "mobile"', 'sys.platform == "other"'
            ),
            self.destination + "AVAILABLE = True\n",
            self.destination + "del sys\n",
            self.destination + "bool = None\n",
            self.destination + "def rebind():\n    global sys\n    sys = None\n",
            self.destination + "from unknown import *\n",
        ):
            with self.subTest(destination=destination), self.assertRaises(ValueError):
                self.copy(destination=destination)
        for source in (
            self.source + "del sys\n",
            self.source + "from unknown import *\n",
            self.source.replace("AVAILABLE = not IS_MOBILE", "AVAILABLE = MISSING"),
            self.source.replace("limit=1", "limit=requires_available()"),
        ):
            with self.subTest(source=source), self.assertRaises(ValueError):
                self.copy(source=source)

    def test_unsupported_namespace_changes_and_future_mismatch_fail_closed(self):
        for statement in (
            "global AVAILABLE\n    AVAILABLE = True",
            "return globals()",
            "return eval('AVAILABLE')",
        ):
            with self.subTest(statement=statement), self.assertRaises(ValueError):
                self.copy(
                    self.source.replace(
                        "    import errno", "    " + statement + "\n    import errno"
                    )
                )
        with self.assertRaises(ValueError):
            self.copy("from __future__ import annotations\n" + self.source)
        with self.assertRaises(ValueError):
            self.copy(self.source.replace("limit=1", "limit=(EXTRA := 1)"))

    def test_anchor_ambiguity_partial_lines_and_changed_comments_fail_closed(self):
        for destination in (
            self.destination.replace("STATUS = 1", "STATUS = 2"),
            self.destination + "STATUS = 1\n",
            self.destination.replace("STATUS = 1", "STATUS = 1; other = 2"),
            self.destination.replace("STATUS = 1", "other = 2; STATUS = 1"),
            self.destination.replace(
                "# Keep this comment with STATUS.", "# Changed comment."
            ),
        ):
            with self.subTest(destination=destination), self.assertRaises(ValueError):
                self.copy(destination=destination)
        replacement = "@functools.cache\ndef anchor(): pass\n"
        with self.assertRaises(ValueError):
            self.copy(
                self.source.replace("STATUS = 1\n", replacement),
                self.destination.replace("STATUS = 1\n", replacement),
            )

    def test_exports_require_complete_canonical_lines(self):
        for exports in (
            self.exports[:1],
            list(reversed(self.exports)),
            self.exports * 2,
            ["other"],
        ):
            with self.subTest(exports=exports), self.assertRaises(ValueError):
                self.copy(exports=exports)
        with self.assertRaises(ValueError):
            self.copy(destination=self.destination + "del __all__\n")
        self.assertNotIn(self.export_line, self.copy(exports=[]))

    def test_cli_hash_pins_receipt_and_concurrent_destination_guard(self):
        with tempfile.TemporaryDirectory() as temp, contextlib.chdir(temp):
            source = pathlib.Path("canonical/Lib/example.py")
            destination = pathlib.Path("Lib/example.py")
            source.parent.mkdir(parents=True)
            destination.parent.mkdir(parents=True)
            source.write_text(self.source)
            destination.write_text(self.destination)
            args = [
                str(source),
                "--functions",
                *self.names,
                "--exports",
                *self.exports,
                "--source-sha256",
                hashlib.sha256(source.read_bytes()).hexdigest(),
                "--destination-sha256",
                "0" * 64,
            ]
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(main(args), 1)
            self.assertEqual(destination.read_text(), self.destination)
            args[-1] = hashlib.sha256(destination.read_bytes()).hexdigest()
            with contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertEqual(main(args), 0)
            receipt = json.loads(output.getvalue())
            self.assertEqual(receipt["functions"], self.names)
            self.assertEqual(destination.read_text(), self.copy())
            destination.write_text(self.destination)

            def concurrent_change(*unused):
                destination.write_text("# changed meanwhile\n")
                return self.copy()

            with patch(
                "update_lib.cmd_migrate.copy_selected_functions", concurrent_change
            ):
                with contextlib.redirect_stderr(io.StringIO()) as errors:
                    self.assertEqual(main(args), 1)
            self.assertIn("Destination changed", errors.getvalue())
            self.assertEqual(destination.read_text(), "# changed meanwhile\n")

    def test_cli_modes_are_exclusive_before_reading_inputs(self):
        for options in (
            ["--assignments", "value"],
            ["--patches", "missing.json"],
            ["--legacy-map", "missing.json"],
            ["--canonical-removals", "missing.json"],
        ):
            with (
                self.subTest(options=options),
                contextlib.redirect_stderr(io.StringIO()) as errors,
            ):
                self.assertEqual(
                    main(
                        ["missing/Lib/example.py", "--functions", *self.names, *options]
                    ),
                    1,
                )
            self.assertIn("cannot combine", errors.getvalue())

    def test_scoped_header_expressions_fail_closed(self):
        for header in (
            "def available(limit=(lambda: requires_available())()):",
            "def available(limit=[requires_available() for _ in (0,)]):",
            "def available(limit={requires_available() for _ in (0,)}):",
            "def available(limit={0: requires_available() for _ in (0,)}):",
            "def available(limit=(requires_available() for _ in (0,))):",
            "def available(*, limit=(lambda: requires_available())()):",
            "def available(limit: (lambda: requires_available())() = 1):",
        ):
            with (
                self.subTest(header=header),
                self.assertRaisesRegex(
                    ValueError, "Scoped expressions in function headers"
                ),
            ):
                self.copy(self.source.replace("def available(limit=1):", header))
        for decorator in (
            "@(lambda: requires_available())()",
            "@[requires_available for _ in (0,)][0]",
        ):
            with (
                self.subTest(decorator=decorator),
                self.assertRaisesRegex(
                    ValueError, "Scoped expressions in function headers"
                ),
            ):
                self.copy(self.source.replace("@functools.cache", decorator))

    def test_nested_classes_fail_closed(self):
        for body in (
            "    class Inner:\n        os = os\n",
            "    def nested():\n        class Inner:\n            os = os\n",
        ):
            with (
                self.subTest(body=body),
                self.assertRaisesRegex(ValueError, "Classes in selected functions"),
            ):
                self.copy(
                    self.source.replace(
                        "    import errno\n", body + "    import errno\n"
                    )
                )


if __name__ == "__main__":
    unittest.main()
