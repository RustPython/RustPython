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
