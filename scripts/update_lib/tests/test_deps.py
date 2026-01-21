"""Tests for deps.py - dependency resolution."""

import pathlib
import tempfile
import unittest

from update_lib.deps import (
    consolidate_test_paths,
    find_tests_importing_module,
    get_data_paths,
    get_lib_paths,
    get_soft_deps,
    get_test_dependencies,
    get_test_paths,
    get_transitive_imports,
    parse_lib_imports,
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
        self.assertEqual(paths, ())


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
        self.assertEqual(imports, {"os", "sys", "collections"})

    def test_from_import(self):
        """Test parsing 'from foo import bar'."""
        code = """
from os import path
from collections.abc import Mapping
from typing import Optional
"""
        imports = parse_lib_imports(code)
        self.assertEqual(imports, {"os", "collections", "typing"})

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


class TestGetTransitiveImports(unittest.TestCase):
    """Tests for get_transitive_imports function."""

    def test_direct_dependency(self):
        """A imports B → B's transitive importers include A."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            lib_dir.mkdir()

            (lib_dir / "a.py").write_text("import b\n")
            (lib_dir / "b.py").write_text("# b module")

            get_transitive_imports.cache_clear()
            result = get_transitive_imports("b", lib_prefix=str(lib_dir))
            self.assertIn("a", result)

    def test_chain_dependency(self):
        """A imports B, B imports C → C's transitive importers include A and B."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            lib_dir.mkdir()

            (lib_dir / "a.py").write_text("import b\n")
            (lib_dir / "b.py").write_text("import c\n")
            (lib_dir / "c.py").write_text("# c module")

            get_transitive_imports.cache_clear()
            result = get_transitive_imports("c", lib_prefix=str(lib_dir))
            self.assertIn("a", result)
            self.assertIn("b", result)

    def test_cycle_handling(self):
        """Handle circular imports without infinite loop."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            lib_dir.mkdir()

            (lib_dir / "a.py").write_text("import b\n")
            (lib_dir / "b.py").write_text("import a\n")  # cycle

            get_transitive_imports.cache_clear()
            # Should not hang or raise
            result = get_transitive_imports("a", lib_prefix=str(lib_dir))
            self.assertIn("b", result)


class TestFindTestsImportingModule(unittest.TestCase):
    """Tests for find_tests_importing_module function."""

    def test_direct_import(self):
        """Test finding tests that directly import a module."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            test_dir = lib_dir / "test"
            test_dir.mkdir(parents=True)

            # Create target module
            (lib_dir / "bar.py").write_text("# bar module")

            # Create test that imports bar
            (test_dir / "test_foo.py").write_text("import bar\n")

            get_transitive_imports.cache_clear()
            find_tests_importing_module.cache_clear()
            result = find_tests_importing_module("bar", lib_prefix=str(lib_dir))
            self.assertIn(test_dir / "test_foo.py", result)

    def test_includes_test_module_itself(self):
        """Test that test_<module>.py IS included in results."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            test_dir = lib_dir / "test"
            test_dir.mkdir(parents=True)

            (lib_dir / "bar.py").write_text("# bar module")
            (test_dir / "test_bar.py").write_text("import bar\n")

            get_transitive_imports.cache_clear()
            find_tests_importing_module.cache_clear()
            result = find_tests_importing_module("bar", lib_prefix=str(lib_dir))
            # test_bar.py IS now included (module's own test is part of impact)
            self.assertIn(test_dir / "test_bar.py", result)

    def test_transitive_import(self):
        """Test finding tests with transitive (indirect) imports."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            test_dir = lib_dir / "test"
            test_dir.mkdir(parents=True)

            # bar.py (target module)
            (lib_dir / "bar.py").write_text("# bar module")

            # baz.py imports bar
            (lib_dir / "baz.py").write_text("import bar\n")

            # test_foo.py imports baz (not bar directly)
            (test_dir / "test_foo.py").write_text("import baz\n")

            get_transitive_imports.cache_clear()
            find_tests_importing_module.cache_clear()
            result = find_tests_importing_module("bar", lib_prefix=str(lib_dir))
            # test_foo.py should be found via transitive dependency
            self.assertIn(test_dir / "test_foo.py", result)

    def test_empty_when_no_importers(self):
        """Test returns empty when no tests import the module."""
        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            test_dir = lib_dir / "test"
            test_dir.mkdir(parents=True)

            (lib_dir / "bar.py").write_text("# bar module")
            (test_dir / "test_unrelated.py").write_text("import os\n")

            get_transitive_imports.cache_clear()
            find_tests_importing_module.cache_clear()
            result = find_tests_importing_module("bar", lib_prefix=str(lib_dir))
            self.assertEqual(result, frozenset())


class TestFindTestsOnlyTestFiles(unittest.TestCase):
    """Tests for filtering to only test_*.py files in output."""

    def test_support_file_not_in_output(self):
        """Support files should not appear in output even if they import target."""
        # Given:
        #   bar.py (target module in Lib/)
        #   helper.py (support file in test/, imports bar)
        #   test_foo.py (test file, imports bar)
        # When: find_tests_importing_module("bar")
        # Then: test_foo.py is included, helper.py is NOT included

        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            test_dir = lib_dir / "test"
            test_dir.mkdir(parents=True)

            (lib_dir / "bar.py").write_text("# bar module")
            # helper.py imports bar directly but doesn't start with test_
            (test_dir / "helper.py").write_text("import bar\n")
            # test_foo.py also imports bar
            (test_dir / "test_foo.py").write_text("import bar\n")

            get_transitive_imports.cache_clear()
            find_tests_importing_module.cache_clear()
            result = find_tests_importing_module("bar", lib_prefix=str(lib_dir))

            # Only test_foo.py should be in results
            self.assertIn(test_dir / "test_foo.py", result)
            # helper.py should be excluded
            self.assertNotIn(test_dir / "helper.py", result)

    def test_transitive_via_support_file(self):
        """Test file importing support file that imports target should be included."""
        # Given:
        #   bar.py (target module in Lib/)
        #   helper.py (support file in test/, imports bar)
        #   test_foo.py (test file, imports helper - NOT bar directly)
        # When: find_tests_importing_module("bar")
        # Then: test_foo.py IS included (via helper.py), helper.py is NOT

        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            test_dir = lib_dir / "test"
            test_dir.mkdir(parents=True)

            (lib_dir / "bar.py").write_text("# bar module")
            # helper.py imports bar
            (test_dir / "helper.py").write_text("import bar\n")
            # test_foo.py imports only helper (not bar directly)
            (test_dir / "test_foo.py").write_text("from test import helper\n")

            get_transitive_imports.cache_clear()
            find_tests_importing_module.cache_clear()
            result = find_tests_importing_module("bar", lib_prefix=str(lib_dir))

            # test_foo.py depends on bar via helper, so it should be included
            self.assertIn(test_dir / "test_foo.py", result)
            # helper.py should be excluded from output
            self.assertNotIn(test_dir / "helper.py", result)

    def test_chain_through_multiple_support_files(self):
        """Test transitive chain through multiple support files."""
        # Given:
        #   bar.py (target)
        #   helper_a.py imports bar
        #   helper_b.py imports helper_a
        #   test_foo.py imports helper_b
        # Then: test_foo.py IS included, helper_a/b are NOT

        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            test_dir = lib_dir / "test"
            test_dir.mkdir(parents=True)

            (lib_dir / "bar.py").write_text("# bar module")
            (test_dir / "helper_a.py").write_text("import bar\n")
            (test_dir / "helper_b.py").write_text("from test import helper_a\n")
            (test_dir / "test_foo.py").write_text("from test import helper_b\n")

            get_transitive_imports.cache_clear()
            find_tests_importing_module.cache_clear()
            result = find_tests_importing_module("bar", lib_prefix=str(lib_dir))

            self.assertIn(test_dir / "test_foo.py", result)
            self.assertNotIn(test_dir / "helper_a.py", result)
            self.assertNotIn(test_dir / "helper_b.py", result)


class TestFindTestsInModuleDirectories(unittest.TestCase):
    """Tests for finding tests inside test_*/ module directories."""

    def test_finds_test_in_module_directory(self):
        """Test files inside test_*/ directories should be found."""
        # Given:
        #   bar.py (target module in Lib/)
        #   test_bar/
        #     __init__.py
        #     test_sub.py (imports bar)
        # When: find_tests_importing_module("bar")
        # Then: test_bar/test_sub.py IS included

        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            test_dir = lib_dir / "test"
            test_bar_dir = test_dir / "test_bar"
            test_bar_dir.mkdir(parents=True)

            (lib_dir / "bar.py").write_text("# bar module")
            (test_bar_dir / "__init__.py").write_text("")
            (test_bar_dir / "test_sub.py").write_text("import bar\n")

            get_transitive_imports.cache_clear()
            find_tests_importing_module.cache_clear()
            result = find_tests_importing_module("bar", lib_prefix=str(lib_dir))

            # test_bar/test_sub.py should be in results
            self.assertIn(test_bar_dir / "test_sub.py", result)

    def test_finds_nested_test_via_support_in_module_directory(self):
        """Transitive deps through support files in module directories."""
        # Given:
        #   bar.py (target)
        #   test_bar/
        #     __init__.py
        #     helper.py (imports bar)
        #     test_sub.py (imports helper via "from test.test_bar import helper")
        # When: find_tests_importing_module("bar")
        # Then: test_bar/test_sub.py IS included, helper.py is NOT

        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            test_dir = lib_dir / "test"
            test_bar_dir = test_dir / "test_bar"
            test_bar_dir.mkdir(parents=True)

            (lib_dir / "bar.py").write_text("# bar module")
            (test_bar_dir / "__init__.py").write_text("")
            (test_bar_dir / "helper.py").write_text("import bar\n")
            (test_bar_dir / "test_sub.py").write_text(
                "from test.test_bar import helper\n"
            )

            get_transitive_imports.cache_clear()
            find_tests_importing_module.cache_clear()
            result = find_tests_importing_module("bar", lib_prefix=str(lib_dir))

            # test_sub.py should be included (via helper)
            self.assertIn(test_bar_dir / "test_sub.py", result)
            # helper.py should NOT be in results (not a test file)
            self.assertNotIn(test_bar_dir / "helper.py", result)

    def test_both_top_level_and_module_directory_tests_found(self):
        """Both top-level test_*.py and test_*/test_*.py should be found."""
        # Given:
        #   bar.py (target)
        #   test_bar.py (top-level, imports bar)
        #   test_bar/
        #     test_sub.py (imports bar)
        # When: find_tests_importing_module("bar")
        # Then: BOTH test_bar.py AND test_bar/test_sub.py are included

        with tempfile.TemporaryDirectory() as tmpdir:
            tmpdir = pathlib.Path(tmpdir)
            lib_dir = tmpdir / "Lib"
            test_dir = lib_dir / "test"
            test_bar_dir = test_dir / "test_bar"
            test_bar_dir.mkdir(parents=True)

            (lib_dir / "bar.py").write_text("# bar module")
            (test_dir / "test_bar.py").write_text("import bar\n")
            (test_bar_dir / "__init__.py").write_text("")
            (test_bar_dir / "test_sub.py").write_text("import bar\n")

            get_transitive_imports.cache_clear()
            find_tests_importing_module.cache_clear()
            result = find_tests_importing_module("bar", lib_prefix=str(lib_dir))

            # Both should be included
            self.assertIn(test_dir / "test_bar.py", result)
            self.assertIn(test_bar_dir / "test_sub.py", result)


class TestConsolidateTestPaths(unittest.TestCase):
    """Tests for consolidate_test_paths function."""

    def test_top_level_test_file(self):
        """Top-level test_*.py -> test_* (without .py)."""
        with tempfile.TemporaryDirectory() as tmpdir:
            test_dir = pathlib.Path(tmpdir)
            test_file = test_dir / "test_foo.py"
            test_file.write_text("# test")

            result = consolidate_test_paths(frozenset({test_file}), test_dir)
            self.assertEqual(result, frozenset({"test_foo"}))

    def test_module_directory_tests_consolidated(self):
        """Multiple files in test_*/ directory -> single directory name."""
        with tempfile.TemporaryDirectory() as tmpdir:
            test_dir = pathlib.Path(tmpdir)
            module_dir = test_dir / "test_sqlite3"
            module_dir.mkdir()
            (module_dir / "test_dbapi.py").write_text("# test")
            (module_dir / "test_backup.py").write_text("# test")

            result = consolidate_test_paths(
                frozenset({module_dir / "test_dbapi.py", module_dir / "test_backup.py"}),
                test_dir,
            )
            self.assertEqual(result, frozenset({"test_sqlite3"}))

    def test_mixed_top_level_and_module_directory(self):
        """Both top-level and module directory tests handled correctly."""
        with tempfile.TemporaryDirectory() as tmpdir:
            test_dir = pathlib.Path(tmpdir)
            # Top-level test
            (test_dir / "test_foo.py").write_text("# test")
            # Module directory tests
            module_dir = test_dir / "test_sqlite3"
            module_dir.mkdir()
            (module_dir / "test_dbapi.py").write_text("# test")
            (module_dir / "test_backup.py").write_text("# test")

            result = consolidate_test_paths(
                frozenset({
                    test_dir / "test_foo.py",
                    module_dir / "test_dbapi.py",
                    module_dir / "test_backup.py",
                }),
                test_dir,
            )
            self.assertEqual(result, frozenset({"test_foo", "test_sqlite3"}))

    def test_empty_input(self):
        """Empty input -> empty frozenset."""
        with tempfile.TemporaryDirectory() as tmpdir:
            test_dir = pathlib.Path(tmpdir)
            result = consolidate_test_paths(frozenset(), test_dir)
            self.assertEqual(result, frozenset())


if __name__ == "__main__":
    unittest.main()
