"""Tests for patch_spec.py - core patch extraction and application."""

import ast
import unittest

from update_lib.patch_spec import (
    COMMENT,
    PatchSpec,
    UtMethod,
    _find_import_insert_line,
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

    def test_class_conditional_method_markers_are_preserved(self):
        code = """class Owner:
    if enabled:
        @unittest.expectedFailure  # TODO: RUSTPYTHON
        def test_value(self):
            return 42
"""
        patches = extract_patches(code)
        self.assertIn("test_value", patches["Owner"])
        clean = code.replace(
            "        @unittest.expectedFailure  # TODO: RUSTPYTHON\n", ""
        )
        result = apply_patches(clean, patches)
        self.assertIn("        @unittest.expectedFailure", result)
        method = list(iter_tests(ast.parse(result)))[0][1]
        self.assertEqual(method.col_offset, 8)
        self.assertEqual(ast.dump(method.body[0]), "Return(value=Constant(value=42))")

    def test_nested_class_markers_do_not_target_module_names(self):
        code = """class Container:
    class Target:
        @unittest.expectedFailure  # TODO: RUSTPYTHON
        def test_value(self):
            pass
def factory():
    class Target:
        @unittest.expectedFailure  # TODO: RUSTPYTHON
        def test_value(self):
            pass
"""
        self.assertEqual(extract_patches(code), {})

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

    def test_comment_confusion(self):
        """
        Test that we only extract our patches when CPython set one of the UT methods,
        that we search for
        """

        code = f"""
class TestFoo(unittest.TestCase):
    @unittest.expectedSuccess # {COMMENT}; reason
    @unittest.expectedFailure
    def test_one(self):
        pass
        """

        patches = extract_patches(code)
        specs = patches["TestFoo"]["test_one"]
        self.assertEqual(len(specs), 1)

        spec = specs[0]
        self.assertEqual(
            spec,
            PatchSpec(ut_method=UtMethod.ExpectedSuccess, cond=None, reason="reason"),
        )


class TestApplyPatches(unittest.TestCase):
    """Tests for apply_patches function."""

    def test_conditional_class_existing_method(self):
        code = """import unittest
if enabled:
    class TestConditional:
        def test_value(self):
            assert 1 + 1 == 2
"""
        result = apply_patches(
            code,
            {
                "TestConditional": {
                    "test_value": [
                        PatchSpec(
                            UtMethod.ExpectedFailureIf, "condition", "known limitation"
                        )
                    ]
                }
            },
        )
        original = list(iter_tests(ast.parse(code)))[0][1]
        updated = list(iter_tests(ast.parse(result)))[0][1]
        self.assertEqual(ast.dump(original.body[0]), ast.dump(updated.body[0]))
        self.assertEqual(updated.col_offset, 8)
        self.assertIn("        @unittest.expectedFailureIf", result)

    def test_conditional_class_inherited_override(self):
        code = """import unittest
class Parent:
    async def test_value(self):
        return 42
if enabled:
    class Child(Parent):
        pass
"""
        result = apply_patches(
            code, {"Child": {"test_value": [PatchSpec(UtMethod.ExpectedSuccess)]}}
        )
        methods = list(iter_tests(ast.parse(result)))
        self.assertEqual([cls.name for cls, _ in methods], ["Parent", "Child"])
        self.assertIsInstance(methods[1][1], ast.AsyncFunctionDef)
        self.assertIn("        return await super().test_value()", result)
        self.assertIn("        @unittest.expectedSuccess", result)

    def test_duplicate_conditional_class_names_are_rejected(self):
        code = """if enabled:
    class Duplicate:
        pass
else:
    class Duplicate:
        pass
"""
        with self.assertRaisesRegex(ValueError, "Ambiguous.*Duplicate"):
            apply_patches(
                code,
                {"Duplicate": {"test_value": [PatchSpec(UtMethod.ExpectedFailure)]}},
            )

    def test_local_class_names_do_not_shadow_module_class(self):
        code = """import unittest
class Target:
    def test_value(self):
        return 1
class Container:
    class Target:
        def test_value(self):
            return 2
def factory():
    class Target:
        def test_value(self):
            return 3
"""
        result = apply_patches(
            code, {"Target": {"test_value": [PatchSpec(UtMethod.ExpectedFailure)]}}
        )
        tree = ast.parse(result)
        targets = [
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.ClassDef) and node.name == "Target"
        ]
        self.assertEqual(
            [len(node.body[0].decorator_list) for node in targets], [1, 0, 0]
        )

    def test_duplicate_conditional_method_names_are_rejected(self):
        code = """class Owner:
    if enabled:
        def test_value(self):
            return 1
    else:
        def test_value(self):
            return 2
"""
        with self.assertRaisesRegex(
            ValueError, "Ambiguous method name: Owner.test_value"
        ):
            apply_patches(
                code, {"Owner": {"test_value": [PatchSpec(UtMethod.ExpectedFailure)]}}
            )

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

    def test_round_trip_with_patches_on_class(self):
        """Test that extracted patches can be re-applied."""
        original = f"""import unittest

@unittest.skipIf(a == b, "{COMMENT}")
@unittest.expectedFailure  # {COMMENT}
class TestFoo(unittest.TestCase):
    ...
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
        self.assertIn("@unittest.skipIf", result)
        self.assertIn(COMMENT, result)


class TestFindImportInsertLine(unittest.TestCase):
    """Tests for _find_import_insert_line function."""

    def test_with_imports(self):
        """Test finding line after imports."""
        code = """import os
import sys

class Foo:
    pass
"""
        tree = ast.parse(code)
        line = _find_import_insert_line(tree)
        self.assertEqual(line, 2)

    def test_no_imports_with_docstring(self):
        """Test fallback to after docstring when no imports."""
        code = '''"""Module docstring."""

class Foo:
    pass
'''
        tree = ast.parse(code)
        line = _find_import_insert_line(tree)
        self.assertEqual(line, 1)

    def test_no_imports_no_docstring(self):
        """Test fallback to line 0 when no imports and no docstring."""
        code = """class Foo:
    pass
"""
        tree = ast.parse(code)
        line = _find_import_insert_line(tree)
        self.assertEqual(line, 0)


if __name__ == "__main__":
    unittest.main()
