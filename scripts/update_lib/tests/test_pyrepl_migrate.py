"""Preserve the existing three pyrepl console mock parser workarounds."""

import ast
import contextlib
import io
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from update_lib.cmd_migrate import (
    patch_single_content,
    preserve_migration_customizations,
)
from update_lib.patch_spec import extract_patches


class TestPyreplConsoleMigration(unittest.TestCase):
    path = Path("Lib/test/test_pyrepl/test_unix_console.py")
    fixtures = Path(__file__).parent / "fixtures"
    local_preamble = (fixtures / "pyrepl_console_local.txt").read_text()
    upstream_preamble = (fixtures / "pyrepl_console_upstream.txt").read_text()
    body = """class TestConsole(TestCase):
    def test_cursor_back_write(self, _os_write):
        self.assertEqual(1, 2)

    def test_new_upstream_case(self):
        self.assertTrue(True)

class TestUnixConsoleInputHook(TestCase):
    def test_input_hook_output_is_cooked(self):
        self.assertEqual(output, b"line1\\r\\nline2\\r\\n")
"""
    prefix = "import unittest\nfrom unittest import TestCase\n"
    local = prefix + local_preamble + body
    upstream = prefix + upstream_preamble + body

    def migrate(self, old, new):
        return preserve_migration_customizations(self.path, old, new)

    def test_preserves_exact_local_preamble_and_canonical_bodies(self):
        old = self.local.replace("self.assertEqual(1, 2)", "self.assertEqual(1, 1)")
        migrated = self.migrate(old, self.upstream)
        self.assertEqual(migrated, self.local)
        self.assertTrue(migrated.endswith(self.body))
        self.assertEqual(self.migrate(migrated, self.upstream), migrated)
        ast.parse(migrated)

    def test_changed_preamble_fails_closed_on_both_sides(self):
        for local in (True, False):
            source = self.local if local else self.upstream
            for changed in (
                source.replace("27394", "27395", 1),
                source.replace('"termios.tcgetattr"', '"termios.changed"', 1),
                source.replace('s + b":"', 's + b";"', 1),
                source.replace("@patch(\n", "@patch( # changed\n", 1),
                source.replace("@force_not_colorized_test_class\n", ""),
            ):
                with self.subTest(local=local, changed=changed):
                    with self.assertRaises(ValueError):
                        self.migrate(
                            changed if local else self.local,
                            self.upstream if local else changed,
                        )

    def test_comment_loss_fails_closed(self):
        with self.assertRaises(ValueError):
            self.migrate(
                self.local.replace("TODO: RUSTPYTHON", "TODO", 1), self.upstream
            )

    def test_missing_duplicate_nested_or_shadowed_owner_fails_closed(self):
        for local in (True, False):
            source = self.local if local else self.upstream
            for changed in (
                source.replace("class TestConsole(", "class OtherConsole("),
                source + "\nclass TestConsole: pass\n",
                "if True:\n"
                + "".join("    " + line for line in source.splitlines(True)),
                source + "\nTestConsole = None\n",
            ):
                with self.subTest(local=local, changed=changed):
                    with self.assertRaises(ValueError):
                        self.migrate(
                            changed if local else self.local,
                            self.upstream if local else changed,
                        )

    def test_missing_duplicate_or_shadowed_helpers_fail_closed(self):
        for name in (
            "__rustpython_patch_terminfo_tparm",
            "__rustpython_patch_termios_tcgetattr",
            "__rustpython_patch_termios_tcsetattr",
        ):
            for changed in (
                self.local.replace("def " + name, "def missing_helper"),
                self.local + f"\ndef {name}(): pass\n",
                self.local + f"\n{name} = None\n",
                self.local + f"\ndel {name}\n",
                self.local + f"\nimport other as {name}\n",
            ):
                with self.subTest(name=name, changed=changed):
                    with self.assertRaises(ValueError):
                        self.migrate(changed, self.upstream)
            with self.assertRaises(ValueError):
                self.migrate(self.local, self.upstream + f"\n{name} = None\n")

    def test_other_paths_are_unchanged(self):
        self.assertEqual(
            preserve_migration_customizations(
                Path("Lib/test/test_other.py"), self.local, self.upstream
            ),
            self.upstream,
        )

    def test_normal_migration_retains_existing_decorator_guard(self):
        guard = (
            '    @unittest.expectedFailureIf(sys.platform == "android", '
            '"TODO: RUSTPYTHON; existing signal handler limitation")\n'
        )
        local = self.local.replace(
            "    def test_cursor_back_write", guard + "    def test_cursor_back_write"
        )
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root)
            source = directory / "canonical.py"
            target = directory / self.path
            source.write_text(self.upstream)
            target.parent.mkdir(parents=True)
            target.write_text(local)
            with (
                patch(
                    "update_lib.cmd_migrate.__file__",
                    directory / "scripts/update_lib/cmd_migrate.py",
                ),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                result = patch_single_content(source, target)
            self.assertEqual(extract_patches(result), extract_patches(local))
            self.assertIn(self.local_preamble, result)
            self.assertIn(self.body.split("class TestUnixConsoleInputHook")[1], result)


if __name__ == "__main__":
    unittest.main()
