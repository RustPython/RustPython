"""Static, tempfile-only tests for explicit package migrations."""

import json
import tempfile
import unittest
from pathlib import Path

from update_lib.migrate_package import migrate_package
from update_lib.patch_spec import iter_patches

MARKED = """import unittest
class Example:
    @unittest.expectedFailure  # TODO: RUSTPYTHON; retained reason
    def test_one(self):
        pass
"""
CANONICAL = """import unittest
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
        (self.src / "test_part.py").write_text(CANONICAL)
        self.dest = self.root / "Lib" / "test" / "package"
        self.dest.parent.mkdir(parents=True)
        self.old = self.dest.parent / "old.py"
        self.old.write_text(MARKED)
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
            list(iter_patches(MARKED)),
        )

    def test_preserves_inherited_override(self):
        (self.src / "test_part.py").write_text(
            CANONICAL.replace("class Example:", "class Base:")
            + "\nclass Example(Base):\n    pass\n"
        )
        self.run_migration()
        text = (self.dest / "test_part.py").read_text()
        self.assertIn("return super().test_one()", text)
        self.assertEqual(list(iter_patches(text)), list(iter_patches(MARKED)))

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
            CANONICAL.replace("test_one", "test_other")
        )
        self.assert_rejected()

    def test_missing_class(self):
        (self.src / "test_part.py").write_text(CANONICAL.replace("Example", "Other"))
        self.assert_rejected()

    def test_ambiguous_class(self):
        (self.src / "test_part.py").write_text(CANONICAL + CANONICAL)
        self.assert_rejected()

    def test_ambiguous_method(self):
        (self.src / "test_part.py").write_text(
            CANONICAL + "    def test_one(self):\n        pass\n"
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
        (self.old.parent / "second.py").write_text(MARKED)
        self.mapping["second.py"] = self.mapping["old.py"]
        self.assert_rejected()
        self.assertEqual((self.old.parent / "second.py").read_text(), MARKED)

    def test_preserves_ordered_markers_from_one_legacy_method(self):
        marked = MARKED.replace(
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
            MARKED + MARKED,
            MARKED + "    def test_one(self):\n        pass\n",
        ):
            with self.subTest(source=source):
                self.old.write_text(source)
                self.assert_rejected()

    def test_unmarked_legacy_retired(self):
        self.old.write_text(CANONICAL)
        self.mapping["old.py"] = {}
        self.run_migration()
        self.assertFalse(self.old.exists())
        self.assertEqual((self.dest / "test_part.py").read_text(), CANONICAL)

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
        outside.write_text(CANONICAL)
        (self.src / "test_part.py").unlink()
        (self.src / "test_part.py").symlink_to(outside)
        self.assert_rejected()

    def test_legacy_symlink_rejected(self):
        outside = self.root / "outside.py"
        outside.write_text(MARKED)
        self.old.unlink()
        self.old.symlink_to(outside)
        self.assert_rejected()

    def test_inheritance_cycle_rejected(self):
        (self.src / "test_part.py").write_text(
            "class Example(Base):\n    pass\nclass Base(Example):\n    pass\n"
        )
        self.assert_rejected()

    def test_canonical_marker_collision_rejected(self):
        (self.src / "test_part.py").write_text(MARKED)
        self.assert_rejected()

    def test_unmarked_syntax_error_prevents_all_writes(self):
        (self.src / "broken.py").write_text("class :\n")
        self.assert_rejected()
