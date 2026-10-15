"""Bounded runtime-patch preservation, independent of interpreter behavior."""

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from update_lib.cmd_migrate import patch_single_content
from update_lib.runtime_patches import (
    EXCEPTION_LOOP,
    JIT_ERROR,
    SYSCONFIG_LOCAL,
    SYSCONFIG_UPSTREAM,
    preserve_runtime_patch,
)


class RuntimePatchTests(unittest.TestCase):
    def test_sysconfig_identity_and_new_content(self):
        path = Path("Lib/sysconfig/__init__.py")
        source = SYSCONFIG_UPSTREAM + "new_final_behavior = True\n"
        result = preserve_runtime_patch(path, SYSCONFIG_LOCAL, source)
        self.assertEqual(result, SYSCONFIG_LOCAL + "new_final_behavior = True\n")
        self.assertEqual(preserve_runtime_patch(path, result, source), result)

    def test_pickle_jit_mapping_and_new_content(self):
        path = Path("Lib/_compat_pickle.py")
        source = "final_mapping = 'ImportCycleError'\n" + EXCEPTION_LOOP
        result = preserve_runtime_patch(path, JIT_ERROR + EXCEPTION_LOOP, source)
        self.assertEqual(
            result, "final_mapping = 'ImportCycleError'\n" + JIT_ERROR + EXCEPTION_LOOP
        )
        self.assertEqual(preserve_runtime_patch(path, result, source), result)

    def test_changed_or_ambiguous_anchors_fail(self):
        cases = [
            (Path("Lib/sysconfig/__init__.py"), SYSCONFIG_LOCAL, SYSCONFIG_UPSTREAM),
            (Path("Lib/_compat_pickle.py"), JIT_ERROR + EXCEPTION_LOOP, EXCEPTION_LOOP),
        ]
        for path, old, source in cases:
            for bad_old in (
                "",
                old + old,
                old.replace("RustPython", "Other").replace("JitError", "Other"),
            ):
                with (
                    self.subTest(path=path, bad_old=bad_old),
                    self.assertRaises(ValueError),
                ):
                    preserve_runtime_patch(path, bad_old, source)
            for bad_source in (
                "",
                source + source,
                source.replace("Python", "Other").replace("excname", "other"),
            ):
                with (
                    self.subTest(path=path, bad_source=bad_source),
                    self.assertRaises(ValueError),
                ):
                    preserve_runtime_patch(path, old, bad_source)

    def test_unrelated_file_unchanged(self):
        self.assertEqual(
            preserve_runtime_patch(Path("Lib/other.py"), JIT_ERROR, "new\n"), "new\n"
        )

    def test_decoy_and_shadowed_sysconfig_anchors_fail(self):
        path = Path("Lib/sysconfig/__init__.py")
        for source in (
            'doc = """' + SYSCONFIG_UPSTREAM + '"""\n',
            SYSCONFIG_UPSTREAM + "def _get_implementation():\n    return 'Other'\n",
            SYSCONFIG_UPSTREAM + "_get_implementation = lambda: 'Other'\n",
            SYSCONFIG_UPSTREAM + "from other import _get_implementation\n",
            SYSCONFIG_UPSTREAM + "import other as _get_implementation\n",
            SYSCONFIG_UPSTREAM + "class _get_implementation: pass\n",
            SYSCONFIG_UPSTREAM + "del _get_implementation\n",
        ):
            with self.subTest(source=source), self.assertRaises(ValueError):
                preserve_runtime_patch(path, SYSCONFIG_LOCAL, source)

    def test_decoy_and_reordered_pickle_anchors_fail(self):
        path = Path("Lib/_compat_pickle.py")
        old = JIT_ERROR + EXCEPTION_LOOP
        with self.assertRaises(ValueError):
            preserve_runtime_patch(path, old, 'doc = """' + EXCEPTION_LOOP + '"""\n')
        with self.assertRaises(ValueError):
            preserve_runtime_patch(path, EXCEPTION_LOOP + JIT_ERROR, EXCEPTION_LOOP)

    def test_similarly_named_nested_files_are_untouched(self):
        for path in (
            "Lib/other/sysconfig/__init__.py",
            "Lib/other/_compat_pickle.py",
            "other/sysconfig/__init__.py",
        ):
            self.assertEqual(preserve_runtime_patch(Path(path), "old", "new"), "new")

    def test_absolute_target_requires_explicit_repository_resolution(self):
        with self.assertRaises(ValueError):
            preserve_runtime_patch(
                Path("/parent/Lib/repository/Lib/sysconfig/__init__.py"),
                SYSCONFIG_LOCAL,
                SYSCONFIG_UPSTREAM,
            )

    def test_migrate_resolves_real_repository_under_lib_parent(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "Lib" / "repository"
            output = root / "Lib" / "sysconfig" / "__init__.py"
            output.parent.mkdir(parents=True)
            output.write_text(SYSCONFIG_LOCAL, encoding="utf-8")
            source = Path(temporary) / "canonical.py"
            source.write_text(SYSCONFIG_UPSTREAM, encoding="utf-8")
            module_file = root / "scripts" / "update_lib" / "cmd_migrate.py"
            with patch("update_lib.cmd_migrate.__file__", str(module_file)):
                self.assertEqual(patch_single_content(source, output), SYSCONFIG_LOCAL)
                outside = (
                    Path(temporary) / "outside" / "Lib" / "sysconfig" / "__init__.py"
                )
                outside.parent.mkdir(parents=True)
                outside.write_text(SYSCONFIG_LOCAL, encoding="utf-8")
                self.assertEqual(
                    patch_single_content(source, outside), SYSCONFIG_UPSTREAM
                )
