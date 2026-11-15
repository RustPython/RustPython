"""Native warning metadata and argument-precedence regressions."""

import _warnings
import unittest
import warnings


class WarningFollowupTests(unittest.TestCase):
    def test_invalid_module_name_normalizes(self):
        with warnings.catch_warnings(record=True) as recorded:
            warnings.simplefilter("always")
            exec("_warnings.warn('metadata')", {"_warnings": _warnings, "__name__": 4})
        self.assertEqual(len(recorded), 1)
        self.assertEqual(recorded[0].module, "<string>")

    def test_module_globals_type_diagnostic(self):
        Namespace = type(
            "Namespace",
            (),
            {"__module__": "reviewpkg", "__qualname__": "Outer.Namespace"},
        )
        with self.assertRaises(TypeError) as caught:
            _warnings.warn_explicit(
                "msg", UserWarning, "foo.py", 1, module_globals=Namespace()
            )
        self.assertEqual(
            str(caught.exception),
            "module_globals must be a dict or a frozendict, not reviewpkg.Outer.Namespace",
        )

    def test_warning_instance_ignores_category(self):
        warning = UserWarning("original instance")
        with warnings.catch_warnings(record=True) as recorded:
            warnings.simplefilter("always")
            _warnings.warn(warning, int)
        self.assertEqual(len(recorded), 1)
        self.assertIs(recorded[0].message, warning)
        self.assertIs(recorded[0].category, UserWarning)


if __name__ == "__main__":
    unittest.main()
