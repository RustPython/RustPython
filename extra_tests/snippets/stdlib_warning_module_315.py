"""Warning module metadata and omitted-module filename filters in Python 3.15."""

import unittest
import warnings


class WarningModuleTests(unittest.TestCase):
    def test_recorded_module(self):
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            warnings.warn_explicit(
                "message", UserWarning, "unrelated.py", 4, module="example.module"
            )
        self.assertEqual(len(caught), 1)
        self.assertEqual(caught[0].module, "example.module")

    def test_omitted_and_explicit_none(self):
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            warnings.warn_explicit("omitted", UserWarning, "/tmp/pkg/mod.py", 1)
            warnings.warn_explicit(
                "none", UserWarning, "/tmp/pkg/mod.py", 1, module=None
            )
        self.assertEqual([str(x.message) for x in caught], ["omitted"])
        self.assertIsNone(caught[0].module)

    def test_filename_component_match(self):
        for filename, pattern in [
            ("/tmp/pkg/mod.py", r"pkg\.mod$"),
            ("/tmp/pkg/__init__.py", r"pkg$"),
            ("<generated>", r"<generated>$"),
            ("", r"<unknown>$"),
        ]:
            with self.subTest(filename=filename):
                with warnings.catch_warnings():
                    warnings.simplefilter("ignore")
                    warnings.filterwarnings("error", module=pattern)
                    with self.assertRaisesRegex(UserWarning, "matched"):
                        warnings.warn_explicit("matched", UserWarning, filename, 1)

    def test_explicit_module_overrides_filename(self):
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("ignore")
            warnings.filterwarnings("always", module=r"pkg\.mod$")
            warnings.warn_explicit(
                "unmatched", UserWarning, "/pkg/mod.py", 1, module="different"
            )
        self.assertEqual(caught, [])

    def test_regular_warn_module(self):
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            warnings.warn("message")
        self.assertEqual(caught[0].module, __name__)

    def test_frozen_module_globals(self):
        namespace = frozendict(__name__="warning_probe", __spec__=None)
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            warnings.warn_explicit(
                "message",
                UserWarning,
                "probe.py",
                1,
                module="warning_probe",
                module_globals=namespace,
            )
        self.assertEqual(caught[0].module, "warning_probe")


if __name__ == "__main__":
    unittest.main()
