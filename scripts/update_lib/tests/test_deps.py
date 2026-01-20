"""Tests for deps.py - dependency resolution."""

import pathlib
import tempfile
import unittest

from update_lib.deps import (
    get_data_paths,
    get_lib_paths,
    get_test_dependencies,
    get_test_paths,
    parse_test_imports,
    resolve_all_paths,
)


class TestParseTestImports(unittest.TestCase):
    """Tests for parse_test_imports function."""

    def test_from_test_import(self):
        """Test parsing 'from test import foo'."""
        code = """
from test import string_tests
from test import lock_tests, other_tests
"""
        imports = parse_test_imports(code)
        self.assertEqual(imports, {"string_tests", "lock_tests", "other_tests"})

    def test_from_test_dot_module(self):
        """Test parsing 'from test.foo import bar'."""
        code = """
from test.string_tests import CommonTest
from test.support import verbose
"""
        imports = parse_test_imports(code)
        self.assertEqual(imports, {"string_tests"})  # support is excluded

    def test_excludes_support(self):
        """Test that 'support' is excluded."""
        code = """
from test import support
from test.support import verbose
"""
        imports = parse_test_imports(code)
        self.assertEqual(imports, set())

    def test_regular_imports_ignored(self):
        """Test that regular imports are ignored."""
        code = """
import os
from collections import defaultdict
from . import helper
"""
        imports = parse_test_imports(code)
        self.assertEqual(imports, set())

    def test_syntax_error_returns_empty(self):
        """Test that syntax errors return empty set."""
        code = "this is not valid python {"
        imports = parse_test_imports(code)
        self.assertEqual(imports, set())


class TestGetLibPaths(unittest.TestCase):
    """Tests for get_lib_paths function."""

    def test_known_dependency(self):
        """Test library with known dependencies."""
        paths = get_lib_paths("datetime", "cpython")
        self.assertEqual(len(paths), 2)
        self.assertIn(pathlib.Path("cpython/Lib/datetime.py"), paths)
        self.assertIn(pathlib.Path("cpython/Lib/_pydatetime.py"), paths)

    def test_default_file(self):
        """Test default to .py file."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            lib_dir.mkdir()
            (lib_dir / "foo.py").write_text("# foo")

            paths = get_lib_paths("foo", str(tmpdir))
            self.assertEqual(paths, [tmpdir / "Lib" / "foo.py"])

    def test_default_directory(self):
        """Test default to directory when file doesn't exist."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            lib_dir.mkdir()
            (lib_dir / "foo").mkdir()

            paths = get_lib_paths("foo", str(tmpdir))
            self.assertEqual(paths, [tmpdir / "Lib" / "foo"])


class TestGetTestPaths(unittest.TestCase):
    """Tests for get_test_paths function."""

    def test_known_dependency(self):
        """Test test with known path override."""
        paths = get_test_paths("regrtest", "cpython")
        self.assertEqual(len(paths), 1)
        self.assertEqual(paths[0], pathlib.Path("cpython/Lib/test/test_regrtest"))

    def test_default_directory(self):
        """Test default to test_name/ directory."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            test_dir = tmpdir / "Lib" / "test"
            test_dir.mkdir(parents=True)
            (test_dir / "test_foo").mkdir()

            paths = get_test_paths("foo", str(tmpdir))
            self.assertEqual(paths, [tmpdir / "Lib" / "test" / "test_foo"])

    def test_default_file(self):
        """Test fallback to test_name.py file."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            test_dir = tmpdir / "Lib" / "test"
            test_dir.mkdir(parents=True)
            (test_dir / "test_foo.py").write_text("# test")

            paths = get_test_paths("foo", str(tmpdir))
            self.assertEqual(paths, [tmpdir / "Lib" / "test" / "test_foo.py"])


class TestGetDataPaths(unittest.TestCase):
    """Tests for get_data_paths function."""

    def test_known_data(self):
        """Test module with known data paths."""
        paths = get_data_paths("regrtest", "cpython")
        self.assertEqual(len(paths), 1)
        self.assertEqual(paths[0], pathlib.Path("cpython/Lib/test/regrtestdata"))

    def test_no_data(self):
        """Test module without data paths."""
        paths = get_data_paths("datetime", "cpython")
        self.assertEqual(paths, [])


class TestGetTestDependencies(unittest.TestCase):
    """Tests for get_test_dependencies function."""

    def test_parse_file_imports(self):
        """Test parsing imports from test file."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            test_dir = tmpdir / "test"
            test_dir.mkdir()

            # Create test file with import
            test_file = test_dir / "test_foo.py"
            test_file.write_text("""
from test import string_tests

class TestFoo:
    pass
""")
            # Create the dependency file
            (test_dir / "string_tests.py").write_text("# string tests")

            result = get_test_dependencies(test_file)
            self.assertEqual(len(result["hard_deps"]), 1)
            self.assertEqual(result["hard_deps"][0], test_dir / "string_tests.py")
            self.assertEqual(result["data"], [])

    def test_nonexistent_path(self):
        """Test nonexistent path returns empty."""
        result = get_test_dependencies(pathlib.Path("/nonexistent/path"))
        self.assertEqual(result, {"hard_deps": [], "data": []})

    def test_transitive_data_dependency(self):
        """Test that data deps are resolved transitively.

        Chain: test_codecencodings_kr -> multibytecodec_support -> cjkencodings
        """
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            test_dir = tmpdir / "test"
            test_dir.mkdir()

            # Create test_codecencodings_kr.py that imports multibytecodec_support
            test_file = test_dir / "test_codecencodings_kr.py"
            test_file.write_text("""
from test import multibytecodec_support

class TestKR:
    pass
""")
            # Create multibytecodec_support.py (the intermediate dependency)
            (test_dir / "multibytecodec_support.py").write_text("# support module")

            # Create cjkencodings directory (the data dependency of multibytecodec_support)
            (test_dir / "cjkencodings").mkdir()

            result = get_test_dependencies(test_file)

            # Should find multibytecodec_support.py as a hard_dep
            self.assertEqual(len(result["hard_deps"]), 1)
            self.assertEqual(
                result["hard_deps"][0], test_dir / "multibytecodec_support.py"
            )

            # Should find cjkencodings as data (from multibytecodec_support's TEST_DEPENDENCIES)
            self.assertEqual(len(result["data"]), 1)
            self.assertEqual(result["data"][0], test_dir / "cjkencodings")


class TestResolveAllPaths(unittest.TestCase):
    """Tests for resolve_all_paths function."""

    def test_datetime(self):
        """Test resolving datetime module."""
        result = resolve_all_paths("datetime", include_deps=False)
        self.assertEqual(len(result["lib"]), 2)
        self.assertIn(pathlib.Path("cpython/Lib/datetime.py"), result["lib"])
        self.assertIn(pathlib.Path("cpython/Lib/_pydatetime.py"), result["lib"])

    def test_regrtest(self):
        """Test resolving regrtest module."""
        result = resolve_all_paths("regrtest", include_deps=False)
        self.assertEqual(result["lib"], [pathlib.Path("cpython/Lib/test/libregrtest")])
        self.assertEqual(
            result["test"], [pathlib.Path("cpython/Lib/test/test_regrtest")]
        )
        self.assertEqual(
            result["data"], [pathlib.Path("cpython/Lib/test/regrtestdata")]
        )


if __name__ == "__main__":
    unittest.main()
