"""Native omitted-module and frozen warning-global boundaries."""

import unittest
import warnings


class WarningModuleTests(unittest.TestCase):
    def test_omitted_and_explicit_none(self):
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            warnings.warn_explicit("omitted", UserWarning, "/tmp/pkg/mod.py", 1)
            warnings.warn_explicit(
                "none", UserWarning, "/tmp/pkg/mod.py", 1, module=None
            )
        self.assertEqual([str(x.message) for x in caught], ["omitted"])
        self.assertIsNone(caught[0].module)

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
