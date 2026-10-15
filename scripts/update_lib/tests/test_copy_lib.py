"""Tests for copy_lib.py - library copying with dependencies."""

import pathlib
import tempfile
import unittest


class TestCopySingle(unittest.TestCase):
    """Tests for _copy_single helper function."""

    def test_copies_file(self):
        """Test copying a single file."""
        from update_lib.cmd_copy_lib import _copy_single

        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)

            src = tmpdir / "source.py"
            src.write_text("content")
            dst = tmpdir / "dest.py"

            _copy_single(src, dst, verbose=False)

            self.assertTrue(dst.exists())
            self.assertEqual(dst.read_text(), "content")

    def test_copies_directory(self):
        """Test copying a directory."""
        from update_lib.cmd_copy_lib import _copy_single

        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)

            src = tmpdir / "source_dir"
            src.mkdir()
            (src / "file.py").write_text("content")
            dst = tmpdir / "dest_dir"

            _copy_single(src, dst, verbose=False)

            self.assertTrue(dst.exists())
            self.assertTrue((dst / "file.py").exists())

    def test_removes_existing_before_copy(self):
        """Test that existing destination is removed before copy."""
        from update_lib.cmd_copy_lib import _copy_single

        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)

            src = tmpdir / "source.py"
            src.write_text("new content")
            dst = tmpdir / "dest.py"
            dst.write_text("old content")

            _copy_single(src, dst, verbose=False)

            self.assertEqual(dst.read_text(), "new content")

    def test_directory_copy_excludes_bytecode_cache_directories(self):
        from update_lib.cmd_copy_lib import _copy_single

        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            src = root / "source"
            dst = root / "destination"
            for relative in ("__pycache__", "package/__pycache__", "assets"):
                (src / relative).mkdir(parents=True)
            files = {
                "__init__.py": b"source\n",
                "package/module.py": b"nested source\n",
                "assets/data.bin": b"\x00\xffasset",
                "assets/__pycache__": b"ordinary file, not a directory",
                "assets/fixture.pyc": b"canonical bytecode fixture",
                "__pycache__/module.cpython-315.pyc": b"stale cache",
                "package/__pycache__/module.cpython-315.pyc": b"nested cache",
            }
            for relative, content in files.items():
                (src / relative).write_bytes(content)

            _copy_single(src, dst, verbose=False)

            expected = {
                relative: content
                for relative, content in files.items()
                if "__pycache__" not in pathlib.Path(relative).parts[:-1]
            }
            actual = {
                path.relative_to(dst).as_posix(): path.read_bytes()
                for path in dst.rglob("*")
                if path.is_file()
            }
            self.assertEqual(actual, expected)
            self.assertFalse((dst / "__pycache__").exists())
            self.assertFalse((dst / "package/__pycache__").exists())
            for relative, content in files.items():
                self.assertEqual((src / relative).read_bytes(), content)


class TestCopyLib(unittest.TestCase):
    """Tests for copy_lib function."""

    def test_raises_on_path_without_lib(self):
        """Test that copy_lib raises ValueError when path doesn't contain /Lib/."""
        from update_lib.cmd_copy_lib import copy_lib

        with self.assertRaises(ValueError) as ctx:
            copy_lib(pathlib.Path("some/path/without/lib.py"))

        self.assertIn("/Lib/", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
