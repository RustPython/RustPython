"""Tests for path.py - path utilities."""

import pathlib
import tempfile
import unittest

from update_lib.path import (
    get_test_files,
    is_lib_path,
    is_test_path,
    lib_to_test_path,
    parse_lib_path,
    test_name_from_path,
)


class TestParseLibPath(unittest.TestCase):
    """Tests for parse_lib_path function."""

    def test_parse_cpython_path(self):
        """Test parsing cpython/Lib/... path."""
        result = parse_lib_path("cpython/Lib/test/test_foo.py")
        self.assertEqual(result, pathlib.Path("Lib/test/test_foo.py"))

    def test_parse_nested_path(self):
        """Test parsing deeply nested path."""
        result = parse_lib_path("/home/user/cpython/Lib/test/test_foo/test_bar.py")
        self.assertEqual(result, pathlib.Path("Lib/test/test_foo/test_bar.py"))

    def test_parse_windows_path(self):
        """Test parsing Windows-style path."""
        result = parse_lib_path("C:\\cpython\\Lib\\test\\test_foo.py")
        self.assertEqual(result, pathlib.Path("Lib/test/test_foo.py"))

    def test_parse_directory(self):
        """Test parsing directory path."""
        result = parse_lib_path("cpython/Lib/test/test_json/")
        self.assertEqual(result, pathlib.Path("Lib/test/test_json/"))

    def test_parse_no_lib_raises(self):
        """Test that path without /Lib/ raises ValueError."""
        with self.assertRaises(ValueError) as ctx:
            parse_lib_path("some/random/path.py")
        self.assertIn("/Lib/", str(ctx.exception))


class TestIsLibPath(unittest.TestCase):
    """Tests for is_lib_path function."""

    def test_lib_path(self):
        """Test detecting Lib/ path."""
        self.assertTrue(is_lib_path(pathlib.Path("Lib/test/test_foo.py")))
        self.assertTrue(is_lib_path(pathlib.Path("./Lib/test/test_foo.py")))

    def test_cpython_path_not_lib(self):
        """Test that cpython/Lib/ is not detected as lib path."""
        self.assertFalse(is_lib_path(pathlib.Path("cpython/Lib/test/test_foo.py")))

    def test_random_path_not_lib(self):
        """Test that random path is not lib path."""
        self.assertFalse(is_lib_path(pathlib.Path("some/other/path.py")))


class TestIsTestPath(unittest.TestCase):
    """Tests for is_test_path function."""

    def test_cpython_test_path(self):
        """Test detecting cpython test path."""
        self.assertTrue(is_test_path(pathlib.Path("cpython/Lib/test/test_foo.py")))

    def test_lib_test_path(self):
        """Test detecting Lib/test path."""
        self.assertTrue(is_test_path(pathlib.Path("Lib/test/test_foo.py")))

    def test_library_path_not_test(self):
        """Test that library path (not test) is not test path."""
        self.assertFalse(is_test_path(pathlib.Path("cpython/Lib/dataclasses.py")))
        self.assertFalse(is_test_path(pathlib.Path("Lib/dataclasses.py")))


class TestLibToTestPath(unittest.TestCase):
    """Tests for lib_to_test_path function."""

    def test_prefers_directory_over_file(self):
        """Test that directory is preferred when both exist."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            # Create structure: tmpdir/Lib/foo.py, tmpdir/Lib/test/test_foo/, tmpdir/Lib/test/test_foo.py
            lib_dir = tmpdir / "Lib"
            lib_dir.mkdir()
            (lib_dir / "foo.py").write_text("# lib")
            test_dir = lib_dir / "test"
            test_dir.mkdir()
            (test_dir / "test_foo").mkdir()
            (test_dir / "test_foo.py").write_text("# test file")

            result = lib_to_test_path(tmpdir / "Lib" / "foo.py")
            # Should prefer directory
            self.assertEqual(result, tmpdir / "Lib" / "test" / "test_foo/")

    def test_falls_back_to_file(self):
        """Test that file is used when directory doesn't exist."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            # Create structure: tmpdir/Lib/foo.py, tmpdir/Lib/test/test_foo.py (no directory)
            lib_dir = tmpdir / "Lib"
            lib_dir.mkdir()
            (lib_dir / "foo.py").write_text("# lib")
            test_dir = lib_dir / "test"
            test_dir.mkdir()
            (test_dir / "test_foo.py").write_text("# test file")

            result = lib_to_test_path(tmpdir / "Lib" / "foo.py")
            # Should fall back to file
            self.assertEqual(result, tmpdir / "Lib" / "test" / "test_foo.py")

    def test_defaults_to_directory_when_neither_exists(self):
        """Test that directory path is returned when neither exists."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            lib_dir.mkdir()
            (lib_dir / "foo.py").write_text("# lib")
            test_dir = lib_dir / "test"
            test_dir.mkdir()
            # Neither test_foo/ nor test_foo.py exists

            result = lib_to_test_path(tmpdir / "Lib" / "foo.py")
            # Should default to directory
            self.assertEqual(result, tmpdir / "Lib" / "test" / "test_foo/")

    def test_lib_path_prefers_directory(self):
        """Test Lib/ path prefers directory when it exists."""
        # This test uses actual Lib/ paths, checking current behavior
        # When neither exists, defaults to directory
        result = lib_to_test_path(pathlib.Path("Lib/nonexistent_module.py"))
        self.assertEqual(result, pathlib.Path("Lib/test/test_nonexistent_module/"))


class TestGetTestFiles(unittest.TestCase):
    """Tests for get_test_files function."""

    def test_single_file(self):
        """Test getting single file."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            test_file = tmpdir / "test.py"
            test_file.write_text("# test")

            files = get_test_files(test_file)
            self.assertEqual(len(files), 1)
            self.assertEqual(files[0], test_file)

    def test_directory(self):
        """Test getting all .py files from directory."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            (tmpdir / "test_a.py").write_text("# a")
            (tmpdir / "test_b.py").write_text("# b")
            (tmpdir / "not_python.txt").write_text("# not python")

            files = get_test_files(tmpdir)
            self.assertEqual(len(files), 2)
            names = [f.name for f in files]
            self.assertIn("test_a.py", names)
            self.assertIn("test_b.py", names)

    def test_nested_directory(self):
        """Test getting .py files from nested directory."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            (tmpdir / "test_a.py").write_text("# a")
            subdir = tmpdir / "subdir"
            subdir.mkdir()
            (subdir / "test_b.py").write_text("# b")

            files = get_test_files(tmpdir)
            self.assertEqual(len(files), 2)


class TestTestNameFromPath(unittest.TestCase):
    """Tests for test_name_from_path function."""

    def test_simple_test_file(self):
        """Test extracting name from simple test file."""
        path = pathlib.Path("Lib/test/test_foo.py")
        self.assertEqual(test_name_from_path(path), "test_foo")

    def test_nested_test_file(self):
        """Test extracting name from nested test directory."""
        path = pathlib.Path("Lib/test/test_ctypes/test_bar.py")
        self.assertEqual(test_name_from_path(path), "test_ctypes.test_bar")

    def test_test_directory(self):
        """Test extracting name from test directory."""
        path = pathlib.Path("Lib/test/test_json")
        self.assertEqual(test_name_from_path(path), "test_json")


if __name__ == "__main__":
    unittest.main()
