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
    _preserve_block,
    main,
    migrate_new_file,
    migrate_package,
    patch_directory,
    patch_file,
    patch_single_content,
    retire_legacy_paths,
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


class TestNewFileAdaptations(unittest.TestCase):
    canonical = (
        "import ctypes\n"
        "api = ctypes.pythonapi.example\n"
        "class Example:\n"
        "    def test_api(self):\n"
        "        assert api() == 1\n"
    )

    @contextlib.contextmanager
    def files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "cpython/Lib/test/test_example.py"
            source.parent.mkdir(parents=True)
            source.write_text(self.canonical)
            repository = root / "rustpython"
            target = repository / "Lib/test/test_example.py"
            document = {
                "target": "Lib/test/test_example.py",
                "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
                "blocks": [
                    [
                        "api = ctypes.pythonapi.example",
                        "api = getattr(ctypes.pythonapi, 'example')",
                    ]
                ],
            }
            with patch(
                "update_lib.cmd_migrate.__file__",
                str(repository / "scripts/update_lib/cmd_migrate.py"),
            ):
                yield source, target, document

    def test_new_file_and_import_binding_scope(self):
        with self.files() as (source, target, document):
            document["blocks"].append(["import ctypes", "import ctypes\nimport sys"])
            migrate_new_file(source, document)
            self.assertEqual(
                target.read_text(),
                self.canonical.replace(
                    "import ctypes", "import ctypes\nimport sys"
                ).replace(
                    "ctypes.pythonapi.example", "getattr(ctypes.pythonapi, 'example')"
                ),
            )
            self.assertEqual(source.read_text(), self.canonical)

    def test_cli_is_explicit_and_existing_target_is_never_overwritten(self):
        with self.files() as (source, target, document):
            metadata = source.parent / "adapters.json"
            metadata.write_text(json.dumps(document))
            self.assertEqual(
                main([str(source), "--new-file-adaptations", str(metadata)]), 0
            )
            previous = target.read_bytes()
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(
                    main([str(source), "--new-file-adaptations", str(metadata)]), 1
                )
                self.assertEqual(
                    main(
                        [
                            str(source),
                            "--new-file-adaptations",
                            str(metadata),
                            "--patches",
                            str(metadata),
                        ]
                    ),
                    1,
                )
            self.assertEqual(target.read_bytes(), previous)

    def test_hash_path_schema_and_block_validation_before_write(self):
        for mutation in (
            {"source_sha256": "0" * 64},
            {"target": "Lib/test/other.py"},
            {"target": "Lib/test/../test_example.py"},
            {"target": "/Lib/test/test_example.py"},
            {"target": "Lib\\test\\test_example.py"},
            {"target": "Lib//test/test_example.py"},
            {"unexpected": True},
            {"blocks": []},
            {"blocks": [["", "x"]]},
            {"blocks": [["not present", "x"]]},
            {"blocks": [["api", "x"]]},
            {
                "blocks": [
                    ["import ctypes", "import sys"],
                    ["import ctypes", "import os"],
                ]
            },
            {"blocks": [["import ctypes", "import sys"], ["ctypes\napi", "sys\napi"]]},
            {"blocks": [["api = ctypes.pythonapi.example", "api = ("]]},
        ):
            with (
                self.subTest(mutation=mutation),
                self.files() as (source, target, document),
            ):
                document.update(mutation)
                with self.assertRaises(ValueError):
                    migrate_new_file(source, document)
                self.assertFalse(target.exists())

    def test_class_assertion_binding_target_and_statement_order_are_protected(self):
        for old, new in (
            ("assert api() == 1", "assert api() == 2"),
            ("class Example", "class Different"),
            ("api = ctypes.pythonapi.example", "other = ctypes.pythonapi.example"),
            ("import ctypes", "import ctypes\nraise RuntimeError"),
            ("class Example:", "@unittest.skip('unsupported')\nclass Example:"),
        ):
            with self.subTest(old=old), self.files() as (source, target, document):
                document["blocks"] = [[old, new]]
                with self.assertRaisesRegex(ValueError, "only imports and bindings"):
                    migrate_new_file(source, document)
                self.assertFalse(target.exists())

    def test_symlink_source_and_destination_parent_are_rejected(self):
        with self.files() as (source, target, document):
            original = source.with_name("original.py")
            source.rename(original)
            source.symlink_to(original)
            with self.assertRaises(ValueError):
                migrate_new_file(source, document)
            self.assertFalse(target.exists())
        with self.files() as (source, target, document):
            target.parent.parent.parent.mkdir(parents=True)
            outside = source.parent / "outside"
            outside.mkdir()
            target.parent.parent.symlink_to(outside, target_is_directory=True)
            with self.assertRaises(ValueError):
                migrate_new_file(source, document)
            self.assertEqual(list(outside.iterdir()), [])


class TestRetireLegacyPaths(unittest.TestCase):
    @contextlib.contextmanager
    def trees(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "cpython" / "Lib" / "test"
            source.mkdir(parents=True)
            local = root / "rustpython"
            (local / "Lib" / "test").mkdir(parents=True)
            with contextlib.chdir(local):
                yield source

    def test_flat_map_retires_only_explicit_paths_without_parsing(self):
        with self.trees() as source:
            mapping = {}
            for name, content in (
                ("bad.py", b"invalid syntax !"),
                ("data.bin", b"\xff\x00"),
            ):
                (source / "new").mkdir(exist_ok=True)
                (source / "new" / name).write_bytes(content)
                Path("Lib/test/new").mkdir(exist_ok=True)
                Path("Lib/test/new", name).write_bytes(content)
                Path("Lib/test", name).write_bytes(content)
                mapping[f"Lib/test/{name}"] = f"Lib/test/new/{name}"
            Path("Lib/test/removed.py").write_text("old = True\n")
            mapping["Lib/test/removed.py"] = None
            Path("Lib/test/original.py").write_text("preserve = True\n")
            Path("map.json").write_text(json.dumps(mapping))

            self.assertEqual(main([str(source), "--legacy-map", "map.json"]), 0)

            for old in mapping:
                self.assertFalse(Path(old).exists())
            self.assertEqual(
                Path("Lib/test/original.py").read_text(), "preserve = True\n"
            )
            self.assertEqual(Path("Lib/test/new/data.bin").read_bytes(), b"\xff\x00")
            self.assertEqual(
                Path("Lib/test/new/bad.py").read_bytes(), b"invalid syntax !"
            )

    def test_late_missing_or_adapted_replacement_preserves_all_legacy_files(self):
        for replacement in (None, b"local adaptation"):
            with self.subTest(replacement=replacement), self.trees() as source:
                Path("Lib/test/first.py").write_text("first")
                Path("Lib/test/second.py").write_text("second")
                (source / "new.py").write_bytes(b"canonical")
                if replacement is not None:
                    Path("Lib/test/new.py").write_bytes(replacement)
                with self.assertRaisesRegex(ValueError, "Replacement is not canonical"):
                    retire_legacy_paths(
                        source,
                        {
                            "Lib/test/first.py": None,
                            "Lib/test/second.py": "Lib/test/new.py",
                        },
                    )
                self.assertTrue(Path("Lib/test/first.py").exists())
                self.assertTrue(Path("Lib/test/second.py").exists())

    def test_file_still_upstream_is_not_retired(self):
        with self.trees() as source:
            (source / "active.py").write_text("active")
            Path("Lib/test/active.py").write_text("active")
            with self.assertRaisesRegex(ValueError, "upstream-absent"):
                retire_legacy_paths(source, {"Lib/test/active.py": None})
            self.assertTrue(Path("Lib/test/active.py").exists())

    def test_outside_paths_and_missing_source_are_rejected(self):
        with self.trees() as source:
            Path("Lib/test/removed.py").write_text("preserve")
            for name in ("outside.py", "Lib/../outside.py", "/tmp/outside.py"):
                with self.subTest(name=name), self.assertRaises(ValueError):
                    retire_legacy_paths(source, {name: None})
            for missing in (
                source / "missing",
                source.parent.parent / "missing/Lib/test",
            ):
                with (
                    self.subTest(missing=missing),
                    self.assertRaisesRegex(ValueError, "Canonical Lib"),
                ):
                    retire_legacy_paths(missing, {"Lib/test/removed.py": None})
                self.assertEqual(Path("Lib/test/removed.py").read_text(), "preserve")


class TestPatchSingleContent(unittest.TestCase):
    """Tests for patch_single_content function."""

    def test_explicit_blocks_preserve_existing_changes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, target = root / "canonical.py", root / "Lib/test/test_example.py"
            target.parent.mkdir(parents=True)
            source.write_text("import unittest\nvalue = 2\n")
            target.write_text("import sys\nimport unittest\nvalue = 1\n")
            document = {
                "target": "Lib/test/test_example.py",
                "patches": {},
                "blocks": [["import unittest\n", "import sys\nimport unittest\n"]],
            }
            with patch(
                "update_lib.cmd_migrate.__file__",
                str(root / "scripts/update_lib/cmd_migrate.py"),
            ):
                self.assertEqual(
                    patch_single_content(source, target), source.read_text()
                )
                result = patch_single_content(source, target, document)
                self.assertEqual(result, "import sys\nimport unittest\nvalue = 2\n")
                for blocks in (
                    None,
                    [[]],
                    [["import unittest\n", "import missing\n"]],
                    [["value = 2\n", "import sys\n"], ["import sys\n", "value = 1\n"]],
                ):
                    with self.subTest(blocks=blocks), self.assertRaises(ValueError):
                        patch_file(
                            source,
                            target,
                            verbose=False,
                            patches_document={**document, "blocks": blocks},
                        )
                    self.assertEqual(
                        target.read_text(), "import sys\nimport unittest\nvalue = 1\n"
                    )

    def test_nested_fragments_and_ambiguous_inputs(self):
        canonical, local = "    value = 1\n", "    value = 2\n"
        self.assertEqual(
            _preserve_block(
                "class X:\n" + local, "class X:\n" + canonical, canonical, local
            ),
            "class X:\n" + local,
        )
        for old, new in (
            (local * 2, canonical),
            (local, canonical * 2),
            ("", canonical),
        ):
            with self.subTest(old=old, new=new), self.assertRaises(ValueError):
                _preserve_block(old, new, canonical, local)

    def test_invalid_result_does_not_write_production_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, target = root / "canonical.py", root / "Lib/example.py"
            target.parent.mkdir()
            source.write_text("x = 1\n")
            target.write_text("x = 'a'\n")
            document = {
                "target": "Lib/example.py",
                "patches": {},
                "blocks": [["1", "'a"]],
            }
            with patch(
                "update_lib.cmd_migrate.__file__",
                str(root / "scripts/update_lib/cmd_migrate.py"),
            ):
                with self.assertRaisesRegex(ValueError, "invalid Python"):
                    patch_file(source, target, verbose=False, patches_document=document)
                self.assertEqual(target.read_text(), "x = 'a'\n")

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

    def test_empty_canonical_fixture_stays_byte_empty(self):
        with tempfile.TemporaryDirectory() as temporary:
            source, target = Path(temporary) / "canonical", Path(temporary) / "local"
            source.mkdir()
            target.mkdir()
            (source / "empty.py").write_bytes(b"")
            (target / "empty.py").write_bytes(b"\n")
            for _ in range(2):
                patch_directory(source, target, verbose=False)
                self.assertEqual((target / "empty.py").read_bytes(), b"")

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

    def test_cli_narrows_unconditional_failure(self):
        def narrow(document):
            document["patches"]["Example"]["test_known"] = [
                {
                    "ut_method": "expectedFailureIf",
                    "cond": "MS_WINDOWS",
                    "reason": "old failure",
                }
            ]

        status, result = self.invoke(narrow)
        self.assertEqual(status, 0)
        self.assertIn(
            'unittest.expectedFailureIf(MS_WINDOWS, "TODO: RUSTPYTHON; old failure")',
            result,
        )
        self.assertIn('unittest.skip("TODO: RUSTPYTHON; retained guard")', result)
        self.assertIn("self.assertEqual(1, 1)", result)

    def test_narrowing_preserves_reason_order_and_existing_targets(self):
        failure = PatchSpec(UtMethod.ExpectedFailure, reason="known failure")
        narrowed = failure._replace(
            ut_method=UtMethod.ExpectedFailureIf, cond="MS_WINDOWS"
        )
        success = PatchSpec(UtMethod.ExpectedSuccess, reason="known success")
        current = {"Example": {"test_known": [failure, success]}}
        for markers in (
            [narrowed, success, narrowed],
            [narrowed._replace(reason="changed"), success],
            [success, narrowed],
            [narrowed._replace(ut_method=UtMethod.SkipIf), success],
            [narrowed._replace(ut_method=UtMethod.ExpectedSuccess), success],
            [narrowed._replace(cond=None), success],
        ):
            document = {
                "target": "Lib/test/test_example.py",
                "patches": patches_to_json({"Example": {"test_known": markers}}),
            }
            with self.subTest(markers=markers), self.assertRaises(ValueError):
                select_remaining_patches(current, document, Path(document["target"]))

        document["patches"] = patches_to_json({"Example": {"test_new": [narrowed]}})
        with self.assertRaises(ValueError):
            select_remaining_patches(current, document, Path(document["target"]))

    def test_conditional_failure_cannot_be_broadened_or_changed(self):
        conditional = PatchSpec(UtMethod.ExpectedFailureIf, cond="MS_WINDOWS")
        current = {"Example": {"test_known": [conditional]}}
        for marker in (
            conditional._replace(ut_method=UtMethod.ExpectedFailure, cond=None),
            conditional._replace(cond="True"),
            conditional._replace(cond="not MS_WINDOWS"),
        ):
            document = {
                "target": "Lib/test/test_example.py",
                "patches": patches_to_json({"Example": {"test_known": [marker]}}),
            }
            with self.subTest(marker=marker), self.assertRaises(ValueError):
                select_remaining_patches(current, document, Path(document["target"]))


class ConditionalMarkerOverrideTests(unittest.TestCase):
    decorator = (
        '    @unittest.expectedFailureIf(not support.is_android, "TODO: RUSTPYTHON")\n'
    )

    def fixture(self):
        old = OVERRIDE_CLEAN.replace(
            "    def test_known", self.decorator + "    def test_known"
        )
        current = extract_patches(old)
        document = {
            "target": "Lib/test/test_example.py",
            "patches": {},
            "retire_conditional": [
                {
                    "class": "Example",
                    "method": "test_known",
                    "decorator": self.decorator,
                }
            ],
        }
        return old, current, document

    def select(self, old, current, document):
        return select_remaining_patches(
            current,
            document,
            Path(document["target"]),
            old_content=old,
        )

    def test_explicit_conditional_retirement(self):
        old, current, document = self.fixture()
        self.assertEqual(self.select(old, current, document), {})

    def test_omitted_conditional_without_authorization_fails(self):
        old, current, document = self.fixture()
        del document["retire_conditional"]
        with self.assertRaises(ValueError):
            self.select(old, current, document)

    def test_absent_duplicate_modified_and_retained_fail_closed(self):
        for change in (
            "absent",
            "duplicate",
            "text",
            "condition",
            "retained",
            "duplicate_source",
        ):
            old, current, document = self.fixture()
            with self.subTest(change=change), self.assertRaises(ValueError):
                if change == "absent":
                    document["retire_conditional"][0]["method"] = "missing"
                elif change == "duplicate":
                    document["retire_conditional"] *= 2
                elif change == "text":
                    document["retire_conditional"][0]["decorator"] = (
                        self.decorator.replace('"', "'")
                    )
                elif change == "condition":
                    old = old.replace("not support.is_android", "support.is_android")
                    current = extract_patches(old)
                elif change == "retained":
                    document["patches"] = patches_to_json(current)
                elif change == "duplicate_source":
                    old = old.replace(self.decorator, self.decorator * 2)
                    current = extract_patches(old)
                self.select(old, current, document)

    def test_other_guards_and_original_source_required(self):
        for replacement in ("skipIf", "expectedFailureIfWindows"):
            old, current, document = self.fixture()
            old = old.replace("expectedFailureIf", replacement)
            document["retire_conditional"][0]["decorator"] = self.decorator.replace(
                "expectedFailureIf", replacement
            )
            with self.subTest(replacement=replacement), self.assertRaises(ValueError):
                self.select(old, extract_patches(old), document)
        old, current, document = self.fixture()
        with self.assertRaises(ValueError):
            self.select(None, current, document)

    def test_cli_conditional_retirement_preserves_remaining_markers(self):
        old = OVERRIDE_OLD.replace(
            "    @unittest.expectedFailure  # TODO: RUSTPYTHON; old failure\n",
            self.decorator,
        )

        def authorize(document):
            document["retire_conditional"] = [
                {
                    "class": "Example",
                    "method": "test_known",
                    "decorator": self.decorator,
                }
            ]

        with patch(__name__ + ".OVERRIDE_OLD", old):
            status, result = MarkerOverrideTests().invoke(authorize)
            self.assertEqual(status, 0)
            self.assertNotIn("expectedFailureIf", result)
            self.assertIn('unittest.skip("TODO: RUSTPYTHON; retained guard")', result)
            status, result = MarkerOverrideTests().invoke()
            self.assertEqual(status, 1)
            self.assertEqual(result, old)


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
