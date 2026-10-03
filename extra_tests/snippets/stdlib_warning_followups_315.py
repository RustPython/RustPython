"""New metadata and diagnostics follow-ups; CPython 3.15 rc2 oracle."""

import _warnings
import unittest
import warnings


class WarningFollowupTests(unittest.TestCase):
    def test_invalid_module_name_normalizes(self):
        class StringSubclass(str):
            pass

        missing = object()
        cases = [
            (missing, "<string>"),
            (4, "<string>"),
            (object(), "<string>"),
            ("pkg.name", "pkg.name"),
            (StringSubclass("pkg.sub"), "pkg.sub"),
        ]
        for name, expected in cases:
            with self.subTest(expected=expected, value_type=type(name).__name__):
                namespace = {"_warnings": _warnings}
                if name is not missing:
                    namespace["__name__"] = name
                with warnings.catch_warnings(record=True) as recorded:
                    warnings.simplefilter("always")
                    exec("_warnings.warn('metadata')", namespace)
                self.assertEqual(len(recorded), 1)
                self.assertEqual(recorded[0].module, expected)
        with warnings.catch_warnings(record=True) as recorded:
            warnings.simplefilter("always")
            exec(
                "_warnings.warn('metadata')", {"_warnings": _warnings, "__name__": None}
            )
        self.assertEqual(recorded, [])

    def test_module_globals_type_diagnostic(self):
        Namespace = type(
            "Namespace",
            (),
            {"__module__": "reviewpkg", "__qualname__": "Outer.Namespace"},
        )
        for value, name in [(42, "int"), (Namespace(), "reviewpkg.Outer.Namespace")]:
            with self.subTest(name=name):
                with self.assertRaises(TypeError) as caught:
                    _warnings.warn_explicit(
                        "msg", UserWarning, "foo.py", 1, module_globals=value
                    )
                self.assertEqual(
                    str(caught.exception),
                    "module_globals must be a dict or a frozendict, not " + name,
                )

    def test_warning_instance_ignores_category(self):
        for invalid_category in (int, 42, object()):
            warning = UserWarning("original instance")
            with self.subTest(category_type=type(invalid_category).__name__):
                with warnings.catch_warnings(record=True) as recorded:
                    warnings.simplefilter("always")
                    _warnings.warn(warning, invalid_category)
                self.assertEqual(len(recorded), 1)
                self.assertIs(recorded[0].message, warning)
                self.assertIs(recorded[0].category, UserWarning)


if __name__ == "__main__":
    unittest.main()
