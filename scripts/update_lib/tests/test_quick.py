"""Tests for quick.py - quick update functionality."""

import pathlib
import tempfile
import unittest
from unittest.mock import patch

from update_lib.path import lib_to_test_path
from update_lib.quick import (
    _expand_shortcut,
    collect_original_methods,
    get_cpython_dir,
    git_commit,
)


class TestGetCpythonDir(unittest.TestCase):
    """Tests for get_cpython_dir function."""

    def test_extract_from_full_path(self):
        """Test extracting cpython dir from full path."""
        path = pathlib.Path("cpython/Lib/dataclasses.py")
        result = get_cpython_dir(path)
        self.assertEqual(result, pathlib.Path("cpython"))

    def test_extract_from_absolute_path(self):
        """Test extracting cpython dir from absolute path."""
        path = pathlib.Path("/some/path/cpython/Lib/test/test_foo.py")
        result = get_cpython_dir(path)
        self.assertEqual(result, pathlib.Path("/some/path/cpython"))

    def test_shortcut_defaults_to_cpython(self):
        """Test that shortcut (no /Lib/) defaults to 'cpython'."""
        path = pathlib.Path("dataclasses")
        result = get_cpython_dir(path)
        self.assertEqual(result, pathlib.Path("cpython"))


class TestExpandShortcut(unittest.TestCase):
    """Tests for _expand_shortcut function."""

    def test_expand_shortcut_to_test_path_integration(self):
        """Test that expanded shortcut works with lib_to_test_path.

        This tests the fix for the bug where args.path was used instead of
        the expanded src_path when calling lib_to_test_path.
        """
        # Simulate the flow in main():
        # 1. User provides "dataclasses"
        # 2. _expand_shortcut converts to "cpython/Lib/dataclasses.py"
        # 3. lib_to_test_path should receive the expanded path, not original

        original_path = pathlib.Path("dataclasses")
        expanded_path = _expand_shortcut(original_path)

        # If cpython/Lib/dataclasses.py exists, it should be expanded
        if expanded_path != original_path:
            # The expanded path should work with lib_to_test_path
            test_path = lib_to_test_path(expanded_path)
            # Should return a valid test path, not raise an error
            self.assertTrue(str(test_path).startswith("cpython/Lib/test/"))

        # The original unexpanded path would fail or give wrong result
        # This is what the bug was - using args.path instead of src_path

    def test_expand_shortcut_file(self):
        """Test expanding a simple name to file path."""
        # This test checks the shortcut works when file exists
        path = pathlib.Path("dataclasses")
        result = _expand_shortcut(path)

        expected_file = pathlib.Path("cpython/Lib/dataclasses.py")
        expected_dir = pathlib.Path("cpython/Lib/dataclasses")

        if expected_file.exists():
            self.assertEqual(result, expected_file)
        elif expected_dir.exists():
            self.assertEqual(result, expected_dir)
        else:
            # If neither exists, should return original
            self.assertEqual(result, path)

    def test_expand_shortcut_already_full_path(self):
        """Test that full paths are not modified."""
        path = pathlib.Path("cpython/Lib/dataclasses.py")
        result = _expand_shortcut(path)
        self.assertEqual(result, path)

    def test_expand_shortcut_nonexistent(self):
        """Test that nonexistent names are returned as-is."""
        path = pathlib.Path("nonexistent_module_xyz")
        result = _expand_shortcut(path)
        self.assertEqual(result, path)


class TestCollectOriginalMethods(unittest.TestCase):
    """Tests for collect_original_methods function."""

    def test_collect_from_file(self):
        """Test collecting methods from single file."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            test_file = tmpdir / "test.py"
            test_file.write_text("""
class TestFoo:
    def test_one(self):
        pass

    def test_two(self):
        pass
""")

            methods = collect_original_methods(test_file)
            self.assertIsInstance(methods, set)
            self.assertEqual(len(methods), 2)
            self.assertIn(("TestFoo", "test_one"), methods)
            self.assertIn(("TestFoo", "test_two"), methods)

    def test_collect_from_directory(self):
        """Test collecting methods from directory."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            (tmpdir / "test_a.py").write_text("""
class TestA:
    def test_a(self):
        pass
""")
            (tmpdir / "test_b.py").write_text("""
class TestB:
    def test_b(self):
        pass
""")

            methods = collect_original_methods(tmpdir)
            self.assertIsInstance(methods, dict)
            self.assertEqual(len(methods), 2)


class TestGitCommit(unittest.TestCase):
    """Tests for git_commit function."""

    @patch("subprocess.run")
    @patch("update_lib.quick.get_cpython_version")
    def test_none_lib_path_not_added(self, mock_version, mock_run):
        """Test that None lib_path doesn't add '.' to git."""
        mock_version.return_value = "v3.14.0"
        mock_run.return_value.returncode = 1  # Has changes

        with tempfile.TemporaryDirectory() as tmpdir:
            test_file = pathlib.Path(tmpdir) / "test.py"
            test_file.write_text("# test")

            git_commit("test", None, test_file, pathlib.Path("cpython"), verbose=False)

            # Check git add was called with only test_file, not "."
            add_call = mock_run.call_args_list[0]
            self.assertIn(str(test_file), add_call[0][0])
            self.assertNotIn(".", add_call[0][0][2:])  # Skip "git" and "add"

    @patch("subprocess.run")
    @patch("update_lib.quick.get_cpython_version")
    def test_none_test_path_not_added(self, mock_version, mock_run):
        """Test that None test_path doesn't add '.' to git."""
        mock_version.return_value = "v3.14.0"
        mock_run.return_value.returncode = 1

        with tempfile.TemporaryDirectory() as tmpdir:
            lib_file = pathlib.Path(tmpdir) / "lib.py"
            lib_file.write_text("# lib")

            git_commit("lib", lib_file, None, pathlib.Path("cpython"), verbose=False)

            add_call = mock_run.call_args_list[0]
            self.assertIn(str(lib_file), add_call[0][0])
            self.assertNotIn(".", add_call[0][0][2:])

    def test_both_none_returns_false(self):
        """Test that both paths None returns False without git operations."""
        # No mocking needed - should return early before any subprocess calls
        result = git_commit("test", None, None, pathlib.Path("cpython"), verbose=False)
        self.assertFalse(result)


if __name__ == "__main__":
    unittest.main()
