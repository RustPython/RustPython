"""Tests for migrate.py - file migration operations."""

import contextlib
import hashlib
import io
import json
import pathlib
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from update_lib.cmd_migrate import (
    CANONICAL_CALL,
    EXCEPTION_LOOP,
    JIT_ERROR,
    LOCAL_CALL,
    SYSCONFIG_LOCAL,
    SYSCONFIG_UPSTREAM,
    copy_support_assignments,
    main,
    migrate_package,
    patch_directory,
    patch_file,
    patch_single_content,
    preserve_migration_customizations,
)
from update_lib.patch_spec import (
    CANONICAL_LOAD_TESTS,
    COMMENT,
    PatchSpec,
    UtMethod,
    extract_patches,
    iter_patches,
    patches_to_json,
    select_remaining_patches,
)


class TestSupportAssignments(unittest.TestCase):
    names = ["_available", "requires_available", "requires_unavailable"]
    exports = names[1:]
    imports = "import os\nimport unittest\n\n"
    export_line = '    "requires_available", "requires_unavailable",\n'
    block = (
        "_available = hasattr(os, 'geteuid') and os.geteuid() == 0\n"
        'requires_available = unittest.skipUnless(_available, "needs privilege")\n'
        'requires_unavailable = unittest.skipIf(_available, "needs no privilege")\n\n\n'
    )
    source = (
        imports
        + '__all__ = [\n    "existing",\n'
        + export_line
        + "]\n\n\n"
        + block
        + "STATUS = 1\n"
    )
    destination = (
        imports
        + '__all__ = [\n    "existing",\n]\n\n\ndef legacy():\n    return "keep π"\n\n\nSTATUS = 1\n'
    )

    def test_canonical_bytes_and_unrelated_helpers_are_preserved(self):
        result = copy_support_assignments(
            self.source, self.destination, self.names, self.exports
        )
        self.assertEqual(
            result,
            self.destination.replace(
                "__all__ = [\n", "__all__ = [\n" + self.export_line
            ).replace("STATUS = 1\n", self.block + "STATUS = 1\n"),
        )

    def test_missing_ambiguous_and_noncontiguous_assignments_are_rejected(self):
        for source in (
            self.source.replace("_available =", "_other ="),
            self.source + "_available = False\n",
            self.source + "def requires_available(): pass\n",
            self.source + "import other as requires_unavailable\n",
            self.source.replace(
                "requires_available =", "unselected = 0\nrequires_available ="
            ),
            self.source.replace("_available =", "unselected = 0; _available =", 1),
            self.source.replace("STATUS = 1", ""),
        ):
            with self.subTest(source=source), self.assertRaises(ValueError):
                copy_support_assignments(
                    source, self.destination, self.names, self.exports
                )

    def test_existing_bindings_dependencies_and_anchor_fail_closed(self):
        for destination in (
            self.destination + "_available = False\n",
            self.destination + "def requires_available(): pass\n",
            self.destination + "import other as requires_unavailable\n",
            self.destination.replace("import os\n", ""),
            self.destination.replace("import os\n", "import os as other\n"),
            self.destination.replace("import os\n", "") + "import os\n",
            self.destination.replace("STATUS = 1", "STATUS = 2"),
            self.destination + "STATUS = 1\n",
        ):
            with self.subTest(destination=destination), self.assertRaises(ValueError):
                copy_support_assignments(
                    self.source, destination, self.names, self.exports
                )

    def test_invalid_exports_are_rejected(self):
        for source, destination, exports in (
            (self.source, self.destination, self.exports + self.exports),
            (self.source, self.destination, ["unselected"]),
            (self.source, self.destination, list(reversed(self.exports))),
            (self.source, self.destination, self.exports[:1]),
            (
                self.source,
                self.destination.replace('"existing"', '"requires_available"'),
                self.exports,
            ),
            (self.source.replace(self.export_line, ""), self.destination, self.exports),
            (
                self.source,
                self.destination.replace('[\n    "existing",\n]', '("existing",)'),
                self.exports,
            ),
            (self.source.replace('"existing"', "3"), self.destination, self.exports),
            (self.source, self.destination.replace('"existing"', "3"), self.exports),
        ):
            with self.subTest(exports=exports), self.assertRaises(ValueError):
                copy_support_assignments(source, destination, self.names, exports)

    def test_dependency_rebinding_deletion_and_builtin_shadowing_are_rejected(self):
        for binding in (
            "os = replacement",
            "del os",
            "import other as os",
            "hasattr = replacement",
            "del hasattr",
            "if condition:\n    unittest = replacement",
        ):
            for source_side in (False, True):
                source, destination = self.source, self.destination
                if source_side:
                    source = source.replace("__all__ = [", binding + "\n__all__ = [")
                else:
                    destination = destination.replace(
                        "__all__ = [", binding + "\n__all__ = ["
                    )
                with (
                    self.subTest(binding=binding, source_side=source_side),
                    self.assertRaisesRegex(ValueError, "Ambiguous dependency binding"),
                ):
                    copy_support_assignments(
                        source, destination, self.names, self.exports
                    )

    def test_decorated_and_partial_line_anchors_are_rejected(self):
        for replacement in (
            "@decorator\ndef anchor(): pass",
            "STATUS = 1; other = 2",
            "other = 2; STATUS = 1",
        ):
            with self.subTest(replacement=replacement), self.assertRaises(ValueError):
                copy_support_assignments(
                    self.source.replace("STATUS = 1", replacement),
                    self.destination.replace("STATUS = 1", replacement),
                    self.names,
                    self.exports,
                )

    def test_wildcard_import_dependencies_are_rejected(self):
        for source, destination in (
            ("from other import *\n" + self.source, self.destination),
            (self.source, "from other import *\n" + self.destination),
        ):
            with self.assertRaisesRegex(ValueError, "Wildcard import"):
                copy_support_assignments(source, destination, self.names, self.exports)

    def test_cli_hash_pins_scope_and_receipt(self):
        with tempfile.TemporaryDirectory() as temp, contextlib.chdir(temp):
            source = pathlib.Path("canonical/Lib/test/support/__init__.py")
            target = pathlib.Path("Lib/test/support/__init__.py")
            source.parent.mkdir(parents=True)
            target.parent.mkdir(parents=True)
            source.write_text(self.source)
            target.write_text(self.destination)
            args = [
                str(source),
                "--support-assignments",
                *self.names,
                "--support-exports",
                *self.exports,
                "--source-sha256",
                hashlib.sha256(source.read_bytes()).hexdigest(),
                "--destination-sha256",
                "0" * 64,
            ]
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(main(args), 1)
                self.assertEqual(
                    main([str(source), "--support-assignments", *self.names]), 1
                )
            self.assertEqual(target.read_text(), self.destination)
            args[-1] = hashlib.sha256(target.read_bytes()).hexdigest()
            with contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertEqual(main(args), 0)
            self.assertIn('"destination_after_sha256"', output.getvalue())
            self.assertIn(self.block, target.read_text())
            for path in (
                "canonical/Lib/os.py",
                "canonical/Lib/test/support/../../os.py",
            ):
                with contextlib.redirect_stderr(io.StringIO()):
                    self.assertEqual(main([path, *args[1:]]), 1)

    def test_selective_mode_rejects_existing_modes_before_reading_metadata(self):
        for mode in ("--patches", "--legacy-map"):
            with contextlib.redirect_stderr(io.StringIO()) as errors:
                self.assertEqual(
                    main(
                        [
                            "missing/Lib/test/support/example.py",
                            "--support-assignments",
                            *self.names,
                            mode,
                            "missing-metadata.json",
                        ]
                    ),
                    1,
                )
            self.assertIn("cannot combine", errors.getvalue())
            with contextlib.redirect_stderr(io.StringIO()) as errors:
                self.assertEqual(
                    main(
                        [
                            "missing/Lib/test/support/example.py",
                            "--support-exports",
                            *self.exports,
                            mode,
                            "missing-metadata.json",
                        ]
                    ),
                    1,
                )
            self.assertIn("require --support-assignments", errors.getvalue())

    def test_default_syntax_error_behavior_is_unchanged(self):
        with patch(
            "update_lib.cmd_migrate.patch_file", side_effect=SyntaxError("invalid")
        ):
            with self.assertRaises(SyntaxError):
                main(["missing/Lib/test/support/example.py"])
        with patch(
            "update_lib.cmd_migrate.migrate_support_assignments",
            side_effect=SyntaxError("invalid"),
        ):
            with contextlib.redirect_stderr(io.StringIO()) as errors:
                self.assertEqual(
                    main(
                        [
                            "missing/Lib/test/support/example.py",
                            "--support-assignments",
                            *self.names,
                        ]
                    ),
                    1,
                )
            self.assertIn("invalid", errors.getvalue())


class TestPatchSingleContent(unittest.TestCase):
    """Tests for patch_single_content function."""

    def migrate_doctest(self, selection=None, explicit=False):
        old = '''"""
>>> raise X, Y  # TODO: RUSTPYTHON; old mismatch # doctest: +EXPECTED_FAILURE
SyntaxError: old result
>>> raise Z  # TODO: RUSTPYTHON; obsolete mismatch # doctest: +EXPECTED_FAILURE
SyntaxError: obsolete result
"""
'''
        canonical = (
            '''"""
>>> raise X, Y
SyntaxError: canonical result
>>> raise Z
SyntaxError: canonical second result
"""
'''
            + CANONICAL_LOAD_TESTS
            + "\n"
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "repository"
            target = root / "Lib/test/test_example.py"
            target.parent.mkdir(parents=True)
            target.write_text(old)
            source = Path(temporary) / "canonical/Lib/test/test_example.py"
            source.parent.mkdir(parents=True)
            source.write_text(canonical)
            args = [str(source)]
            if explicit:
                metadata = Path(temporary) / "selection.json"
                metadata.write_text(
                    json.dumps(
                        {
                            "target": "Lib/test/test_example.py",
                            "patches": {},
                            "doctests": selection,
                        }
                    )
                )
                args += ["--patches", str(metadata)]
            with (
                contextlib.chdir(root),
                patch(
                    "update_lib.cmd_migrate.__file__",
                    str(root / "scripts/update_lib/cmd_migrate.py"),
                ),
                contextlib.redirect_stdout(io.StringIO()),
                contextlib.redirect_stderr(io.StringIO()),
            ):
                result = main(args)
            return result, target.read_text(), old, canonical

    def test_doctest_default_requires_explicit_selection(self):
        status, result, old, _ = self.migrate_doctest()
        self.assertEqual(status, 1)
        self.assertEqual(result, old)

    def test_doctest_cli_selection_keeps_only_reviewed_guard(self):
        status, result, _, _ = self.migrate_doctest(
            {"raise X, Y\n": "known mismatch"}, True
        )
        self.assertEqual(status, 0)
        self.assertEqual(result.count("+EXPECTED_FAILURE"), 1)
        self.assertIn("SyntaxError: canonical result", result)
        self.assertIn("SyntaxError: canonical second result", result)
        self.assertNotIn("obsolete mismatch", result)

    def test_doctest_empty_selection_keeps_canonical_unchanged(self):
        status, result, _, canonical = self.migrate_doctest({}, True)
        self.assertEqual(status, 0)
        self.assertEqual(result, canonical)

    def test_doctest_invalid_selection_writes_nothing(self):
        for selection in (
            None,
            [],
            {"raise unknown\n": "unrecognized"},
            {"raise X, Y\n": "line\nbreak"},
        ):
            with self.subTest(selection=selection):
                status, result, old, _ = self.migrate_doctest(selection, True)
                self.assertEqual(status, 1)
                self.assertEqual(result, old)

    def test_patch_with_no_existing_file(self):
        """Test patching when lib file doesn't exist."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)

            # Create source file
            src_path = tmpdir / "src.py"
            src_path.write_text("""import unittest

class TestFoo(unittest.TestCase):
    def test_one(self):
        pass
""")

            # Non-existent lib path
            lib_path = tmpdir / "lib.py"

            result = patch_single_content(src_path, lib_path)

            # Should return source content unchanged
            self.assertIn("def test_one(self):", result)
            self.assertNotIn(COMMENT, result)

    def test_patch_with_existing_patches(self):
        """Test patching preserves existing patches."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)

            # Create source file (new version)
            src_path = tmpdir / "src.py"
            src_path.write_text("""import unittest

class TestFoo(unittest.TestCase):
    def test_one(self):
        pass

    def test_two(self):
        pass
""")

            # Create lib file with existing patch
            lib_path = tmpdir / "lib.py"
            lib_path.write_text(f"""import unittest

class TestFoo(unittest.TestCase):
    # {COMMENT}
    @unittest.expectedFailure
    def test_one(self):
        pass
""")

            result = patch_single_content(src_path, lib_path)

            # Should have patch on test_one
            self.assertIn("@unittest.expectedFailure", result)
            self.assertIn(COMMENT, result)
            # Should have test_two from source
            self.assertIn("def test_two(self):", result)


class TestPatchFile(unittest.TestCase):
    """Tests for patch_file function."""

    def test_patch_file_creates_output(self):
        """Test that patch_file writes output file."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)

            # Create source file
            src_path = tmpdir / "src.py"
            src_path.write_text("""import unittest

class TestFoo(unittest.TestCase):
    def test_one(self):
        pass
""")

            # Output path
            lib_path = tmpdir / "Lib" / "test.py"

            patch_file(src_path, lib_path, verbose=False)

            # File should exist
            self.assertTrue(lib_path.exists())
            content = lib_path.read_text()
            self.assertIn("def test_one(self):", content)

    def test_patch_file_preserves_patches(self):
        """Test that patch_file preserves existing patches."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)

            # Create source file
            src_path = tmpdir / "src.py"
            src_path.write_text("""import unittest

class TestFoo(unittest.TestCase):
    def test_one(self):
        pass
""")

            # Create existing lib file with patch
            lib_path = tmpdir / "lib.py"
            lib_path.write_text(f"""import unittest

class TestFoo(unittest.TestCase):
    # {COMMENT}
    @unittest.expectedFailure
    def test_one(self):
        pass
""")

            patch_file(src_path, lib_path, verbose=False)

            content = lib_path.read_text()
            self.assertIn("@unittest.expectedFailure", content)


class TestPatchDirectory(unittest.TestCase):
    """Tests for patch_directory function."""

    def test_excludes_cache_directories_but_preserves_canonical_assets(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            source = root / "source"
            destination = root / "destination"
            for relative in ("__pycache__", "nested/__pycache__", "assets"):
                (source / relative).mkdir(parents=True)
            files = {
                "module.py": b"# canonical source\n",
                "__pycache__/module.cpython-315.pyc": b"stale cache",
                "nested/__pycache__/module.cpython-315.pyc": b"nested cache",
                "assets/__pycache__": b"ordinary canonical file",
                "assets/fixture.pyc": b"canonical bytecode fixture",
            }
            for relative, content in files.items():
                (source / relative).write_bytes(content)
            patch_directory(source, destination, verbose=False)
            expected = {
                relative: content
                for relative, content in files.items()
                if "__pycache__" not in pathlib.Path(relative).parts[:-1]
            }
            actual = {
                path.relative_to(destination).as_posix(): path.read_bytes()
                for path in destination.rglob("*")
                if path.is_file()
            }
            self.assertEqual(actual, expected)
            for relative, content in files.items():
                self.assertEqual((source / relative).read_bytes(), content)

    def test_patch_directory_all_files(self):
        """Test that patch_directory processes all .py files."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)

            # Create source directory with files
            src_dir = tmpdir / "src"
            src_dir.mkdir()
            (src_dir / "test_a.py").write_text("# test_a")
            (src_dir / "test_b.py").write_text("# test_b")
            (src_dir / "subdir").mkdir()
            (src_dir / "subdir" / "test_c.py").write_text("# test_c")

            # Output directory
            lib_dir = tmpdir / "lib"

            patch_directory(src_dir, lib_dir, verbose=False)

            # All files should exist
            self.assertTrue((lib_dir / "test_a.py").exists())
            self.assertTrue((lib_dir / "test_b.py").exists())
            self.assertTrue((lib_dir / "subdir" / "test_c.py").exists())

    def test_patch_directory_preserves_patches(self):
        """Test that patch_directory preserves patches in existing files."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)

            # Create source directory
            src_dir = tmpdir / "src"
            src_dir.mkdir()
            (src_dir / "test_a.py").write_text("""import unittest

class TestA(unittest.TestCase):
    def test_one(self):
        pass
""")

            # Create lib directory with patched file
            lib_dir = tmpdir / "lib"
            lib_dir.mkdir()
            (lib_dir / "test_a.py").write_text(f"""import unittest

class TestA(unittest.TestCase):
    # {COMMENT}
    @unittest.expectedFailure
    def test_one(self):
        pass
""")

            patch_directory(src_dir, lib_dir, verbose=False)

            content = (lib_dir / "test_a.py").read_text()
            self.assertIn("@unittest.expectedFailure", content)


class RuntimePatchTests(unittest.TestCase):
    def test_sysconfig_identity_and_new_content(self):
        path = Path("Lib/sysconfig/__init__.py")
        source = SYSCONFIG_UPSTREAM + "new_final_behavior = True\n"
        result = preserve_migration_customizations(path, SYSCONFIG_LOCAL, source)
        self.assertEqual(result, SYSCONFIG_LOCAL + "new_final_behavior = True\n")
        self.assertEqual(
            preserve_migration_customizations(path, result, source), result
        )

    def test_pickle_jit_mapping_and_new_content(self):
        path = Path("Lib/_compat_pickle.py")
        source = "final_mapping = 'ImportCycleError'\n" + EXCEPTION_LOOP
        result = preserve_migration_customizations(
            path, JIT_ERROR + EXCEPTION_LOOP, source
        )
        self.assertEqual(
            result, "final_mapping = 'ImportCycleError'\n" + JIT_ERROR + EXCEPTION_LOOP
        )
        self.assertEqual(
            preserve_migration_customizations(path, result, source), result
        )

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
                    preserve_migration_customizations(path, bad_old, source)
            for bad_source in (
                "",
                source + source,
                source.replace("Python", "Other").replace("excname", "other"),
            ):
                with (
                    self.subTest(path=path, bad_source=bad_source),
                    self.assertRaises(ValueError),
                ):
                    preserve_migration_customizations(path, old, bad_source)

    def test_unrelated_file_unchanged(self):
        self.assertEqual(
            preserve_migration_customizations(Path("Lib/other.py"), JIT_ERROR, "new\n"),
            "new\n",
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
                preserve_migration_customizations(path, SYSCONFIG_LOCAL, source)

    def test_decoy_and_reordered_pickle_anchors_fail(self):
        path = Path("Lib/_compat_pickle.py")
        old = JIT_ERROR + EXCEPTION_LOOP
        with self.assertRaises(ValueError):
            preserve_migration_customizations(
                path, old, 'doc = """' + EXCEPTION_LOOP + '"""\n'
            )
        with self.assertRaises(ValueError):
            preserve_migration_customizations(
                path, EXCEPTION_LOOP + JIT_ERROR, EXCEPTION_LOOP
            )

    def test_similarly_named_nested_files_are_untouched(self):
        for path in (
            "Lib/other/sysconfig/__init__.py",
            "Lib/other/_compat_pickle.py",
            "other/sysconfig/__init__.py",
        ):
            self.assertEqual(
                preserve_migration_customizations(Path(path), "old", "new"), "new"
            )

    def test_absolute_target_requires_explicit_repository_resolution(self):
        with self.assertRaises(ValueError):
            preserve_migration_customizations(
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


CALL_PATH = Path("Lib/test/test_cmd_line.py")
CALL_TEMPLATE = """class CmdLineTest:
    def test_empty_PYTHONPATH_issue16309(self):
        result = CALL
        self.assertEqual(result, 'new final assertion')
"""
CALL_OLD = CALL_TEMPLATE.replace("CALL", LOCAL_CALL).replace(
    "new final assertion", "old assertion"
)
CALL_CANONICAL = CALL_TEMPLATE.replace("CALL", CANONICAL_CALL)


class CallPatchTests(unittest.TestCase):
    def test_final_body_and_roundtrip(self):
        result = preserve_migration_customizations(CALL_PATH, CALL_OLD, CALL_CANONICAL)
        self.assertEqual(result, CALL_TEMPLATE.replace("CALL", LOCAL_CALL))
        self.assertIn("new final assertion", result)
        self.assertEqual(
            preserve_migration_customizations(CALL_PATH, result, CALL_CANONICAL), result
        )

    def test_missing_duplicate_owners_and_calls_fail(self):
        for source in (
            CALL_CANONICAL.replace("CmdLineTest", "Other"),
            CALL_CANONICAL + CALL_CANONICAL,
            CALL_CANONICAL.replace(
                "        result = ", "        " + CANONICAL_CALL + "\n        result = "
            ),
            CALL_CANONICAL.replace("test_empty_PYTHONPATH_issue16309", "other"),
        ):
            with self.subTest(source=source), self.assertRaises(ValueError):
                preserve_migration_customizations(CALL_PATH, CALL_OLD, source)

    def test_changed_keywords_and_decoy_fail(self):
        for old in (
            CALL_OLD.replace('RUSTPYTHONPATH=""', 'RUSTPYTHONPATH="changed"'),
            CALL_CANONICAL,
            'doc = """' + CALL_OLD + '"""\n',
        ):
            with self.subTest(old=old), self.assertRaises(ValueError):
                preserve_migration_customizations(CALL_PATH, old, CALL_CANONICAL)
        with self.assertRaises(ValueError):
            preserve_migration_customizations(
                CALL_PATH,
                CALL_OLD,
                CALL_CANONICAL.replace('PYTHONPATH=""', 'PYTHONPATH="changed"'),
            )

    def test_unrelated_file_unchanged(self):
        self.assertEqual(
            preserve_migration_customizations(Path("Lib/test/other.py"), "old", "new"),
            "new",
        )


OVERRIDE_CLEAN = """import unittest
class Example(unittest.TestCase):
    def test_known(self):
        self.assertEqual(1, 1)
    def test_guarded(self):
        self.assertEqual(2, 2)
"""
OVERRIDE_OLD = OVERRIDE_CLEAN.replace(
    "    def test_known",
    "    @unittest.expectedFailure  # TODO: RUSTPYTHON; old failure\n    def test_known",
).replace(
    "    def test_guarded",
    '    @unittest.skip("TODO: RUSTPYTHON; retained guard")\n    def test_guarded',
)


class MarkerOverrideTests(unittest.TestCase):
    def invoke(self, mutation=None, directory=False, override=True, legacy=False):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "repository"
            output = root / "Lib" / "test" / "test_example.py"
            output.parent.mkdir(parents=True)
            output.write_text(OVERRIDE_OLD)
            source = Path(temporary) / "cpython" / "Lib" / "test" / "test_example.py"
            source.parent.mkdir(parents=True)
            source.write_text(OVERRIDE_CLEAN)
            markers = patches_to_json(extract_patches(OVERRIDE_OLD))
            del markers["Example"]["test_known"]
            document = {"target": "Lib/test/test_example.py", "patches": markers}
            if mutation:
                mutation(document)
            metadata = Path(temporary) / "remaining.json"
            metadata.write_text(json.dumps(document))
            args = [str(source.parent if directory else source)]
            if override:
                args += ["--patches", str(metadata)]
            if legacy:
                args += ["--legacy-map", str(metadata)]
            with (
                contextlib.chdir(root),
                patch(
                    "update_lib.cmd_migrate.__file__",
                    str(root / "scripts/update_lib/cmd_migrate.py"),
                ),
                contextlib.redirect_stdout(io.StringIO()),
                contextlib.redirect_stderr(io.StringIO()),
            ):
                status = main(args)
            return status, output.read_text()

    def test_cli_override_removes_only_requested_expected_failure(self):
        status, result = self.invoke()
        self.assertEqual(status, 0)
        self.assertNotIn("expectedFailure", result)
        self.assertIn('unittest.skip("TODO: RUSTPYTHON; retained guard")', result)
        self.assertIn("self.assertEqual(1, 1)", result)
        self.assertIn("self.assertEqual(2, 2)", result)

    def test_default_migration_preserves_all_markers(self):
        status, result = self.invoke(override=False)
        self.assertEqual(status, 0)
        self.assertIn("expectedFailure", result)
        self.assertIn("retained guard", result)

    def test_invalid_target_does_not_write(self):
        status, result = self.invoke(
            lambda document: document.update(target="Lib/test/other.py")
        )
        self.assertEqual(status, 1)
        self.assertEqual(result, OVERRIDE_OLD)

    def test_removing_skip_does_not_write(self):
        status, result = self.invoke(
            lambda document: document["patches"]["Example"].clear()
        )
        self.assertEqual(status, 1)
        self.assertEqual(result, OVERRIDE_OLD)

    def test_directory_override_does_not_write(self):
        status, result = self.invoke(directory=True)
        self.assertEqual(status, 1)
        self.assertEqual(result, OVERRIDE_OLD)

    def test_legacy_combination_does_not_write(self):
        status, result = self.invoke(legacy=True)
        self.assertEqual(status, 1)
        self.assertEqual(result, OVERRIDE_OLD)

    def test_added_changed_or_reordered_markers_fail(self):
        failure = PatchSpec(UtMethod.ExpectedFailure, reason="known failure")
        success = PatchSpec(UtMethod.ExpectedSuccess, reason="known success")
        current = {"Example": {"test_known": [failure, success]}}
        for markers in (
            [failure, success, failure],
            [PatchSpec(UtMethod.ExpectedFailure, reason="changed"), success],
            [success, failure],
        ):
            document = {
                "target": "Lib/test/test_example.py",
                "patches": patches_to_json({"Example": {"test_known": markers}}),
            }
            with self.subTest(markers=markers), self.assertRaises(ValueError):
                select_remaining_patches(current, document, Path(document["target"]))


PACKAGE_MARKED = """import unittest
class Example:
    @unittest.expectedFailure  # TODO: RUSTPYTHON; retained reason
    def test_one(self):
        pass
"""
PACKAGE_CANONICAL = """import unittest
class Example:
    def test_one(self):
        pass
"""


class TestMigratePackage(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.src = self.root / "canonical" / "package"
        self.src.mkdir(parents=True)
        (self.src / "__init__.py").write_text("# canonical package\n")
        (self.src / "test_part.py").write_text(PACKAGE_CANONICAL)
        self.dest = self.root / "Lib" / "test" / "package"
        self.dest.parent.mkdir(parents=True)
        self.old = self.dest.parent / "old.py"
        self.old.write_text(PACKAGE_MARKED)
        self.manifest = self.root / "map.json"
        self.mapping = {"old.py": {"Example": "test_part.py"}}

    def run_migration(self):
        self.manifest.write_text(json.dumps(self.mapping))
        migrate_package(self.src, self.dest, self.manifest)

    def assert_rejected(self):
        before = self.old.read_bytes()
        with self.assertRaises((ValueError, FileNotFoundError, SyntaxError)):
            self.run_migration()
        self.assertFalse(self.dest.exists())
        self.assertEqual(self.old.read_bytes(), before)

    def test_preserves_marker_and_canonical_asset(self):
        (self.src / "asset.dat").write_bytes(b"\x00canonical")
        self.run_migration()
        self.assertFalse(self.old.exists())
        self.assertEqual((self.dest / "asset.dat").read_bytes(), b"\x00canonical")
        self.assertEqual(
            list(iter_patches((self.dest / "test_part.py").read_text())),
            list(iter_patches(PACKAGE_MARKED)),
        )

    def test_preserves_inherited_override(self):
        (self.src / "test_part.py").write_text(
            PACKAGE_CANONICAL.replace("class Example:", "class Base:")
            + "\nclass Example(Base):\n    pass\n"
        )
        self.run_migration()
        text = (self.dest / "test_part.py").read_text()
        self.assertIn("return super().test_one()", text)
        self.assertEqual(list(iter_patches(text)), list(iter_patches(PACKAGE_MARKED)))

    def test_excludes_cache_directories_but_preserves_canonical_assets(self):
        for relative in ("__pycache__", "nested/__pycache__", "assets"):
            (self.src / relative).mkdir(parents=True)
        files = {
            "__pycache__/module.cpython-315.pyc": b"stale cache",
            "nested/__pycache__/module.cpython-315.pyc": b"nested cache",
            "assets/__pycache__": b"ordinary canonical file",
            "assets/fixture.pyc": b"canonical bytecode fixture",
        }
        for relative, content in files.items():
            (self.src / relative).write_bytes(content)
        self.run_migration()
        self.assertFalse((self.dest / "__pycache__").exists())
        self.assertFalse((self.dest / "nested" / "__pycache__").exists())
        for relative in ("assets/__pycache__", "assets/fixture.pyc"):
            self.assertEqual((self.dest / relative).read_bytes(), files[relative])
        for relative, content in files.items():
            self.assertEqual((self.src / relative).read_bytes(), content)

    def test_missing_method(self):
        (self.src / "test_part.py").write_text(
            PACKAGE_CANONICAL.replace("test_one", "test_other")
        )
        self.assert_rejected()

    def test_missing_class(self):
        (self.src / "test_part.py").write_text(
            PACKAGE_CANONICAL.replace("Example", "Other")
        )
        self.assert_rejected()

    def test_ambiguous_class(self):
        (self.src / "test_part.py").write_text(PACKAGE_CANONICAL + PACKAGE_CANONICAL)
        self.assert_rejected()

    def test_ambiguous_method(self):
        (self.src / "test_part.py").write_text(
            PACKAGE_CANONICAL + "    def test_one(self):\n        pass\n"
        )
        self.assert_rejected()

    def test_missing_destination(self):
        self.mapping["old.py"]["Example"] = "missing.py"
        self.assert_rejected()

    def test_unused_mapping(self):
        self.mapping["old.py"]["Unused"] = "test_part.py"
        self.assert_rejected()

    def test_unmapped_marker(self):
        self.mapping["old.py"] = {}
        self.assert_rejected()

    def test_target_traversal(self):
        self.mapping["old.py"]["Example"] = "../outside.py"
        self.assert_rejected()

    def test_legacy_traversal(self):
        self.mapping["../old.py"] = self.mapping.pop("old.py")
        self.assert_rejected()

    def test_duplicate_marker(self):
        (self.old.parent / "second.py").write_text(PACKAGE_MARKED)
        self.mapping["second.py"] = self.mapping["old.py"]
        self.assert_rejected()
        self.assertEqual((self.old.parent / "second.py").read_text(), PACKAGE_MARKED)

    def test_preserves_ordered_markers_from_one_legacy_method(self):
        marked = PACKAGE_MARKED.replace(
            "    def test_one",
            '    @unittest.skip("TODO: RUSTPYTHON; retained skip")\n    def test_one',
        )
        self.old.write_text(marked)
        self.run_migration()
        result = (self.dest / "test_part.py").read_text()
        self.assertEqual(list(iter_patches(result)), list(iter_patches(marked)))
        self.assertLess(result.index("expectedFailure"), result.index("unittest.skip"))

    def test_duplicate_legacy_owner_is_rejected(self):
        for source in (
            PACKAGE_MARKED + PACKAGE_MARKED,
            PACKAGE_MARKED + "    def test_one(self):\n        pass\n",
        ):
            with self.subTest(source=source):
                self.old.write_text(source)
                self.assert_rejected()

    def test_unmarked_legacy_retired(self):
        self.old.write_text(PACKAGE_CANONICAL)
        self.mapping["old.py"] = {}
        self.run_migration()
        self.assertFalse(self.old.exists())
        self.assertEqual((self.dest / "test_part.py").read_text(), PACKAGE_CANONICAL)

    def test_legacy_doctest_guards_prevent_all_writes(self):
        self.old.write_text(
            '"""\n'
            ">>> raise ValueError  # TODO: RUSTPYTHON; mismatch # doctest: +EXPECTED_FAILURE\n"
            "ValueError\n"
            '"""\n' + PACKAGE_MARKED
        )
        self.assert_rejected()

    def test_validation_failure_after_valid_entry_writes_nothing(self):
        self.mapping["missing.py"] = {}
        self.assert_rejected()

    def test_existing_destination_is_not_overwritten(self):
        self.dest.mkdir()
        with self.assertRaises(ValueError):
            self.run_migration()
        self.assertTrue(self.old.exists())
        self.assertEqual(list(self.dest.iterdir()), [])

    def test_source_escape_rejected(self):
        outside = self.root / "outside.py"
        outside.write_text(PACKAGE_CANONICAL)
        (self.src / "test_part.py").unlink()
        (self.src / "test_part.py").symlink_to(outside)
        self.assert_rejected()

    def test_legacy_symlink_rejected(self):
        outside = self.root / "outside.py"
        outside.write_text(PACKAGE_MARKED)
        self.old.unlink()
        self.old.symlink_to(outside)
        self.assert_rejected()

    def test_inheritance_cycle_rejected(self):
        (self.src / "test_part.py").write_text(
            "class Example(Base):\n    pass\nclass Base(Example):\n    pass\n"
        )
        self.assert_rejected()

    def test_canonical_marker_collision_rejected(self):
        (self.src / "test_part.py").write_text(PACKAGE_MARKED)
        self.assert_rejected()

    def test_unmarked_syntax_error_prevents_all_writes(self):
        (self.src / "broken.py").write_text("class :\n")
        self.assert_rejected()


if __name__ == "__main__":
    unittest.main()
