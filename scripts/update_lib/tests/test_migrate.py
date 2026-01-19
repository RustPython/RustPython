"""Tests for migrate.py - file migration operations."""

import pathlib
import tempfile
import unittest

from update_lib.migrate import (
    patch_directory,
    patch_file,
    patch_single_content,
)
from update_lib.patch_spec import COMMENT


class TestPatchSingleContent(unittest.TestCase):
    """Tests for patch_single_content function."""

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


if __name__ == "__main__":
    unittest.main()
