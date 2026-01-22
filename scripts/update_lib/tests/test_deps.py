"""Tests for deps.py - dependency resolution."""

import pathlib
import tempfile
import unittest

from update_lib.deps import (
    get_lib_paths,
    get_soft_deps,
    get_test_dependencies,
    get_test_paths,
    parse_lib_imports,
    parse_test_imports,
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
            self.assertEqual(paths, (tmpdir / "Lib" / "foo.py",))

    def test_default_directory(self):
        """Test default to directory when file doesn't exist."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            lib_dir.mkdir()
            (lib_dir / "foo").mkdir()

            paths = get_lib_paths("foo", str(tmpdir))
            self.assertEqual(paths, (tmpdir / "Lib" / "foo",))


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
            self.assertEqual(paths, (tmpdir / "Lib" / "test" / "test_foo",))

    def test_default_file(self):
        """Test fallback to test_name.py file."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            test_dir = tmpdir / "Lib" / "test"
            test_dir.mkdir(parents=True)
            (test_dir / "test_foo.py").write_text("# test")

            paths = get_test_paths("foo", str(tmpdir))
            self.assertEqual(paths, (tmpdir / "Lib" / "test" / "test_foo.py",))


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


class TestParseLibImports(unittest.TestCase):
    """Tests for parse_lib_imports function."""

    def test_import_statement(self):
        """Test parsing 'import foo'."""
        code = """
import os
import sys
import collections.abc
"""
        imports = parse_lib_imports(code)
        self.assertEqual(imports, {"os", "sys", "collections.abc"})

    def test_from_import(self):
        """Test parsing 'from foo import bar'."""
        code = """
from os import path
from collections.abc import Mapping
from typing import Optional
"""
        imports = parse_lib_imports(code)
        self.assertEqual(imports, {"os", "collections.abc", "typing"})

    def test_mixed_imports(self):
        """Test mixed import styles."""
        code = """
import sys
from os import path
from collections import defaultdict
import functools
"""
        imports = parse_lib_imports(code)
        self.assertEqual(imports, {"sys", "os", "collections", "functools"})

    def test_syntax_error_returns_empty(self):
        """Test that syntax errors return empty set."""
        code = "this is not valid python {"
        imports = parse_lib_imports(code)
        self.assertEqual(imports, set())

    def test_relative_import_skipped(self):
        """Test that relative imports (no module) are skipped."""
        code = """
from . import foo
from .. import bar
"""
        imports = parse_lib_imports(code)
        self.assertEqual(imports, set())


class TestGetSoftDeps(unittest.TestCase):
    """Tests for get_soft_deps function."""

    def test_with_temp_files(self):
        """Test soft deps detection with temp files."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            lib_dir.mkdir()

            # Create a module that imports another module
            (lib_dir / "foo.py").write_text("""
import bar
from baz import something
""")
            # Create the imported modules
            (lib_dir / "bar.py").write_text("# bar module")
            (lib_dir / "baz.py").write_text("# baz module")

            soft_deps = get_soft_deps("foo", str(tmpdir))
            self.assertEqual(soft_deps, {"bar", "baz"})

    def test_skips_self(self):
        """Test that module doesn't include itself in soft_deps."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            lib_dir.mkdir()

            # Create a module that imports itself (circular)
            (lib_dir / "foo.py").write_text("""
import foo
import bar
""")
            (lib_dir / "bar.py").write_text("# bar module")

            soft_deps = get_soft_deps("foo", str(tmpdir))
            self.assertNotIn("foo", soft_deps)
            self.assertIn("bar", soft_deps)

    def test_filters_nonexistent(self):
        """Test that nonexistent modules are filtered out."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            lib_dir.mkdir()

            # Create a module that imports nonexistent module
            (lib_dir / "foo.py").write_text("""
import bar
import nonexistent
""")
            (lib_dir / "bar.py").write_text("# bar module")
            # nonexistent.py is NOT created

            soft_deps = get_soft_deps("foo", str(tmpdir))
            self.assertEqual(soft_deps, {"bar"})


class TestDircmpIsSame(unittest.TestCase):
    """Tests for _dircmp_is_same function."""

    def test_identical_directories(self):
        """Test that identical directories return True."""
        import filecmp

        from update_lib.deps import _dircmp_is_same

        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            dir1 = tmpdir / "dir1"
            dir2 = tmpdir / "dir2"
            dir1.mkdir()
            dir2.mkdir()

            (dir1 / "file.py").write_text("content")
            (dir2 / "file.py").write_text("content")

            dcmp = filecmp.dircmp(dir1, dir2)
            self.assertTrue(_dircmp_is_same(dcmp))

    def test_different_files(self):
        """Test that directories with different files return False."""
        import filecmp

        from update_lib.deps import _dircmp_is_same

        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            dir1 = tmpdir / "dir1"
            dir2 = tmpdir / "dir2"
            dir1.mkdir()
            dir2.mkdir()

            (dir1 / "file.py").write_text("content1")
            (dir2 / "file.py").write_text("content2")

            dcmp = filecmp.dircmp(dir1, dir2)
            self.assertFalse(_dircmp_is_same(dcmp))

    def test_nested_identical(self):
        """Test that nested identical directories return True."""
        import filecmp

        from update_lib.deps import _dircmp_is_same

        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            dir1 = tmpdir / "dir1"
            dir2 = tmpdir / "dir2"
            (dir1 / "sub").mkdir(parents=True)
            (dir2 / "sub").mkdir(parents=True)

            (dir1 / "sub" / "file.py").write_text("content")
            (dir2 / "sub" / "file.py").write_text("content")

            dcmp = filecmp.dircmp(dir1, dir2)
            self.assertTrue(_dircmp_is_same(dcmp))

    def test_nested_different(self):
        """Test that nested directories with differences return False."""
        import filecmp

        from update_lib.deps import _dircmp_is_same

        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            dir1 = tmpdir / "dir1"
            dir2 = tmpdir / "dir2"
            (dir1 / "sub").mkdir(parents=True)
            (dir2 / "sub").mkdir(parents=True)

            (dir1 / "sub" / "file.py").write_text("content1")
            (dir2 / "sub" / "file.py").write_text("content2")

            dcmp = filecmp.dircmp(dir1, dir2)
            self.assertFalse(_dircmp_is_same(dcmp))


if __name__ == "__main__":
    unittest.main()
