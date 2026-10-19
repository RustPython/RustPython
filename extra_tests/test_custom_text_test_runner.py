"""Infrastructure regression tests; the nested cases deliberately fail."""

import io
import json
import pathlib
import sys
import tempfile
import types
import unittest

from custom_text_test_runner import CustomTextTestRunner


class RunnerResultTests(unittest.TestCase):
    def test_records_parent_and_subtest_outcomes(self):
        class Cases(unittest.TestCase):
            def test_pass(self):
                for x in range(2):
                    with self.subTest(x=x):
                        self.assertTrue(True)

            def test_fail_error(self):
                with self.subTest(x="fail"):
                    self.fail("subfailure")
                with self.subTest(x="error"):
                    raise ValueError("suberror")

            def test_skip(self):
                with self.subTest(x="skip"):
                    self.skipTest("subskip")

            def test_parent_error(self):
                with self.subTest(x="fail"):
                    self.fail("before parent")
                raise RuntimeError("parent error")

            def test_parent_failure(self):
                with self.subTest(x="error"):
                    raise ValueError("before parent")
                self.fail("parent failure")

            def test_nested(self):
                with self.subTest(outer=1):
                    with self.subTest(inner=2):
                        self.assertTrue(True)

            def test_plain(self):
                pass

            @unittest.expectedFailure
            def test_unexpected(self):
                with self.subTest(x=1):
                    pass

            @unittest.expectedFailure
            def test_expected(self):
                with self.subTest(x=1):
                    self.fail("expected")

            def test_skip_after_failure(self):
                with self.subTest(x=1):
                    self.fail("before skip")
                self.skipTest("parent skip")

            def test_cleanup_error(self):
                def cleanup():
                    raise ValueError("cleanup")

                self.addCleanup(cleanup)
                self.fail("parent")

        class Other(unittest.TestCase):
            def test_other(self):
                pass

        names = [
            "test_pass",
            "test_fail_error",
            "test_skip",
            "test_parent_error",
            "test_parent_failure",
            "test_nested",
            "test_plain",
            "test_plain",
            "test_unexpected",
            "test_expected",
            "test_skip_after_failure",
            "test_cleanup_error",
        ]

        def suite():
            return unittest.TestSuite(
                [Cases(names[0]), Other("test_other"), *(Cases(n) for n in names[1:])]
            )

        standard = unittest.TestResult()
        suite().run(standard)
        with tempfile.TemporaryDirectory() as temp:
            log = io.StringIO()
            path = pathlib.Path(temp) / "results.json"
            actual = CustomTextTestRunner(
                stream=log, verbosity=2, results_file_path=str(path)
            ).run(suite())
            for attr in (
                "testsRun",
                "failures",
                "errors",
                "skipped",
                "expectedFailures",
                "unexpectedSuccesses",
            ):
                a, b = getattr(actual, attr), getattr(standard, attr)
                assert (a if isinstance(a, int) else len(a)) == (
                    b if isinstance(b, int) else len(b)
                ), attr
            data = json.loads(path.read_text())
            cases = [c for s in data["suites"].values() for c in s["cases"].values()]
            assert len(cases) == len(names) + 1, (len(cases), len(names))
            assert all(c["result"] is not None for c in cases)
            assert sum(c["result"] == "passed" for c in cases) == 5, cases
            assert data["num_passed"] == 5, data
            assert data["num_failed"] == 6, data
            assert data["num_skipped"] == 1, data
            subs = [sub for c in cases for sub in c["subtests"]]
            assert len(subs) == 11, subs
            for t, e in actual.errors + actual.failures:
                assert t.id() in log.getvalue(), t.id()
                assert e.strip() in log.getvalue(), e
            assert sum(c["id"].endswith(".test_plain") for c in cases) == 2
            assert data["num_expected_failures"] == 1

    def test_fixture_outcomes_without_start_test(self):
        for boundary in (
            "setUpModule",
            "tearDownModule",
            "setUpClass",
            "tearDownClass",
            "class_cleanup",
            "module_cleanup",
        ):
            for skipped in (False, True):
                with (
                    self.subTest(boundary=boundary, skipped=skipped),
                    tempfile.TemporaryDirectory() as temp,
                ):
                    module = types.ModuleType("runner_fixture_probe")

                    def problem():
                        if skipped:
                            raise unittest.SkipTest("fixture skip")
                        raise ValueError("fixture error")

                    class Fixture(unittest.TestCase):
                        def test_body(self):
                            pass

                    Fixture.__module__ = module.__name__
                    if boundary in ("setUpModule", "tearDownModule"):
                        setattr(module, boundary, problem)
                    elif boundary == "module_cleanup":
                        module.setUpModule = lambda: unittest.addModuleCleanup(problem)
                    elif boundary == "class_cleanup":

                        @classmethod
                        def setup(cls):
                            cls.addClassCleanup(problem)

                        Fixture.setUpClass = setup
                    else:
                        setattr(Fixture, boundary, classmethod(lambda cls: problem()))
                    sys.modules[module.__name__] = module
                    try:
                        expected = unittest.TestResult()
                        unittest.TestSuite([Fixture("test_body")]).run(expected)
                        path = pathlib.Path(temp) / "results.json"
                        actual = CustomTextTestRunner(
                            stream=io.StringIO(),
                            verbosity=2,
                            results_file_path=str(path),
                        ).run(unittest.TestSuite([Fixture("test_body")]))
                        self.assertEqual(actual.testsRun, expected.testsRun)
                        self.assertEqual(len(actual.errors), len(expected.errors))
                        self.assertEqual(len(actual.skipped), len(expected.skipped))
                        cases = [
                            case
                            for suite in json.loads(path.read_text())["suites"].values()
                            for case in suite["cases"].values()
                        ]
                        fixtures = [
                            case
                            for case in cases
                            if case["id"] != Fixture("test_body").id()
                        ]
                        self.assertEqual(len(fixtures), 1)
                        self.assertIn(
                            {
                                "class_cleanup": "tearDownClass",
                                "module_cleanup": "tearDownModule",
                            }.get(boundary, boundary),
                            fixtures[0]["id"],
                        )
                        self.assertEqual(
                            fixtures[0]["result"], "skipped" if skipped else "error"
                        )
                    finally:
                        del sys.modules[module.__name__]

    def test_interruption_keeps_original_exception(self):
        class Interrupted(unittest.TestCase):
            def test_body(self):
                raise KeyboardInterrupt("owned mock interruption")

        with tempfile.TemporaryDirectory() as temp:
            path = pathlib.Path(temp) / "results.json"
            with self.assertRaisesRegex(KeyboardInterrupt, "owned mock interruption"):
                CustomTextTestRunner(
                    stream=io.StringIO(), results_file_path=str(path)
                ).run(unittest.TestSuite([Interrupted("test_body")]))
            data = json.loads(path.read_text())
            self.assertEqual(
                data["num_passed"] + data["num_failed"] + data["num_skipped"], 0
            )


if __name__ == "__main__":
    unittest.main()
