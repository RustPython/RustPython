"""Tests for patch_spec.py - core patch extraction and application."""

import ast
import unittest

from update_lib.patch_spec import (
    COMMENT,
    PatchSpec,
    UtMethod,
    apply_patches,
    extract_patches,
    iter_tests,
)


class TestIterTests(unittest.TestCase):
    """Tests for iter_tests function."""

    def test_iter_tests_simple(self):
        """Test iterating over test methods in a class."""
        code = """
class TestFoo(unittest.TestCase):
    def test_one(self):
        pass

    def test_two(self):
        pass
"""
        tree = ast.parse(code)
        results = list(iter_tests(tree))
        self.assertEqual(len(results), 2)
        self.assertEqual(results[0][0].name, "TestFoo")
        self.assertEqual(results[0][1].name, "test_one")
        self.assertEqual(results[1][1].name, "test_two")

    def test_iter_tests_multiple_classes(self):
        """Test iterating over multiple test classes."""
        code = """
class TestFoo(unittest.TestCase):
    def test_foo(self):
        pass

class TestBar(unittest.TestCase):
    def test_bar(self):
        pass
"""
        tree = ast.parse(code)
        results = list(iter_tests(tree))
        self.assertEqual(len(results), 2)
        self.assertEqual(results[0][0].name, "TestFoo")
        self.assertEqual(results[1][0].name, "TestBar")

    def test_iter_tests_async(self):
        """Test iterating over async test methods."""
        code = """
class TestAsync(unittest.TestCase):
    async def test_async(self):
        pass
"""
        tree = ast.parse(code)
        results = list(iter_tests(tree))
        self.assertEqual(len(results), 1)
        self.assertEqual(results[0][1].name, "test_async")


class TestExtractPatches(unittest.TestCase):
    """Tests for extract_patches function."""

    def test_extract_expected_failure(self):
        """Test extracting @unittest.expectedFailure decorator."""
        code = f"""
class TestFoo(unittest.TestCase):
    # {COMMENT}
    @unittest.expectedFailure
    def test_one(self):
        pass
"""
        patches = extract_patches(code)
        self.assertIn("TestFoo", patches)
        self.assertIn("test_one", patches["TestFoo"])
        specs = patches["TestFoo"]["test_one"]
        self.assertEqual(len(specs), 1)
        self.assertEqual(specs[0].ut_method, UtMethod.ExpectedFailure)

    def test_extract_expected_failure_inline_comment(self):
        """Test extracting expectedFailure with inline comment."""
        code = f"""
class TestFoo(unittest.TestCase):
    @unittest.expectedFailure  # {COMMENT}
    def test_one(self):
        pass
"""
        patches = extract_patches(code)
        self.assertIn("TestFoo", patches)
        self.assertIn("test_one", patches["TestFoo"])

    def test_extract_skip_with_reason(self):
        """Test extracting @unittest.skip with reason."""
        code = f'''
class TestFoo(unittest.TestCase):
    @unittest.skip("{COMMENT}; not implemented")
    def test_one(self):
        pass
'''
        patches = extract_patches(code)
        self.assertIn("TestFoo", patches)
        specs = patches["TestFoo"]["test_one"]
        self.assertEqual(specs[0].ut_method, UtMethod.Skip)
        self.assertIn("not implemented", specs[0].reason)

    def test_extract_skip_if(self):
        """Test extracting @unittest.skipIf decorator."""
        code = f'''
class TestFoo(unittest.TestCase):
    @unittest.skipIf(sys.platform == "win32", "{COMMENT}; windows issue")
    def test_one(self):
        pass
'''
        patches = extract_patches(code)
        specs = patches["TestFoo"]["test_one"]
        self.assertEqual(specs[0].ut_method, UtMethod.SkipIf)
        # ast.unparse normalizes quotes to single quotes
        self.assertIn("sys.platform", specs[0].cond)
        self.assertIn("win32", specs[0].cond)

    def test_no_patches_without_comment(self):
        """Test that decorators without COMMENT are not extracted."""
        code = """
class TestFoo(unittest.TestCase):
    @unittest.expectedFailure
    def test_one(self):
        pass
"""
        patches = extract_patches(code)
        self.assertEqual(patches, {})

    def test_multiple_patches_same_method(self):
        """Test extracting multiple decorators on same method."""
        code = f'''
class TestFoo(unittest.TestCase):
    # {COMMENT}
    @unittest.expectedFailure
    @unittest.skip("{COMMENT}; reason")
    def test_one(self):
        pass
'''
        patches = extract_patches(code)
        specs = patches["TestFoo"]["test_one"]
        self.assertEqual(len(specs), 2)


class TestApplyPatches(unittest.TestCase):
    """Tests for apply_patches function."""

    def test_apply_expected_failure(self):
        """Test applying @unittest.expectedFailure."""
        code = """import unittest

class TestFoo(unittest.TestCase):
    def test_one(self):
        pass
"""
        patches = {
            "TestFoo": {"test_one": [PatchSpec(UtMethod.ExpectedFailure, None, "")]}
        }
        result = apply_patches(code, patches)
        self.assertIn("@unittest.expectedFailure", result)
        self.assertIn(COMMENT, result)

    def test_apply_skip_with_reason(self):
        """Test applying @unittest.skip with reason."""
        code = """import unittest

class TestFoo(unittest.TestCase):
    def test_one(self):
        pass
"""
        patches = {
            "TestFoo": {"test_one": [PatchSpec(UtMethod.Skip, None, "not ready")]}
        }
        result = apply_patches(code, patches)
        self.assertIn("@unittest.skip", result)
        self.assertIn("not ready", result)

    def test_apply_skip_if(self):
        """Test applying @unittest.skipIf."""
        code = """import unittest

class TestFoo(unittest.TestCase):
    def test_one(self):
        pass
"""
        patches = {
            "TestFoo": {
                "test_one": [
                    PatchSpec(UtMethod.SkipIf, "sys.platform == 'win32'", "windows")
                ]
            }
        }
        result = apply_patches(code, patches)
        self.assertIn("@unittest.skipIf", result)
        self.assertIn('sys.platform == "win32"', result)

    def test_apply_preserves_existing_decorators(self):
        """Test that existing decorators are preserved."""
        code = """import unittest

class TestFoo(unittest.TestCase):
    @some_decorator
    def test_one(self):
        pass
"""
        patches = {
            "TestFoo": {"test_one": [PatchSpec(UtMethod.ExpectedFailure, None, "")]}
        }
        result = apply_patches(code, patches)
        self.assertIn("@some_decorator", result)
        self.assertIn("@unittest.expectedFailure", result)

    def test_apply_inherited_method(self):
        """Test applying patch to inherited method (creates override)."""
        code = """import unittest

class TestFoo(unittest.TestCase):
    pass
"""
        patches = {
            "TestFoo": {
                "test_inherited": [PatchSpec(UtMethod.ExpectedFailure, None, "")]
            }
        }
        result = apply_patches(code, patches)
        self.assertIn("def test_inherited(self):", result)
        self.assertIn("return super().test_inherited()", result)

    def test_apply_adds_unittest_import(self):
        """Test that unittest import is added if missing."""
        code = """import sys

class TestFoo:
    def test_one(self):
        pass
"""
        patches = {
            "TestFoo": {"test_one": [PatchSpec(UtMethod.ExpectedFailure, None, "")]}
        }
        result = apply_patches(code, patches)
        # Should add unittest import after existing imports
        self.assertIn("import unittest", result)

    def test_apply_no_duplicate_import(self):
        """Test that unittest import is not duplicated."""
        code = """import unittest

class TestFoo(unittest.TestCase):
    def test_one(self):
        pass
"""
        patches = {
            "TestFoo": {"test_one": [PatchSpec(UtMethod.ExpectedFailure, None, "")]}
        }
        result = apply_patches(code, patches)
        # Count occurrences of 'import unittest'
        count = result.count("import unittest")
        self.assertEqual(count, 1)


class TestPatchSpec(unittest.TestCase):
    """Tests for PatchSpec class."""

    def test_as_decorator_expected_failure(self):
        """Test generating expectedFailure decorator string."""
        spec = PatchSpec(UtMethod.ExpectedFailure, None, "reason")
        decorator = spec.as_decorator()
        self.assertIn("@unittest.expectedFailure", decorator)
        self.assertIn(COMMENT, decorator)
        self.assertIn("reason", decorator)

    def test_as_decorator_skip(self):
        """Test generating skip decorator string."""
        spec = PatchSpec(UtMethod.Skip, None, "not ready")
        decorator = spec.as_decorator()
        self.assertIn("@unittest.skip", decorator)
        self.assertIn("not ready", decorator)

    def test_as_decorator_skip_if(self):
        """Test generating skipIf decorator string."""
        spec = PatchSpec(UtMethod.SkipIf, "condition", "reason")
        decorator = spec.as_decorator()
        self.assertIn("@unittest.skipIf", decorator)
        self.assertIn("condition", decorator)


class TestRoundTrip(unittest.TestCase):
    """Tests for extract -> apply round trip."""

    def test_round_trip_expected_failure(self):
        """Test that extracted patches can be re-applied."""
        original = f"""import unittest

class TestFoo(unittest.TestCase):
    # {COMMENT}
    @unittest.expectedFailure
    def test_one(self):
        pass
"""
        # Extract patches
        patches = extract_patches(original)

        # Apply to clean code
        clean = """import unittest

class TestFoo(unittest.TestCase):
    def test_one(self):
        pass
"""
        result = apply_patches(clean, patches)

        # Should have the decorator
        self.assertIn("@unittest.expectedFailure", result)
        self.assertIn(COMMENT, result)


if __name__ == "__main__":
    unittest.main()
