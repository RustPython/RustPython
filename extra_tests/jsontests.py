import os
import unittest

from custom_text_test_runner import CustomTextTestRunner as Runner
from test.libregrtest.findtests import findtests

testnames = findtests()
# idk why this fixes the hanging, if it does
testnames.remove("test_importlib")
testnames.insert(0, "test_importlib")


def loadTestsOrSkip(loader, name):
    try:
        return loader.loadTestsFromName(name)
    except unittest.SkipTest as exc:
        # from _make_skipped_test from unittest/loader.py
        @unittest.skip(str(exc))
        def testSkipped(self):
            pass

        attrs = {name: testSkipped}
        TestClass = type("ModuleSkipped", (unittest.TestCase,), attrs)
        return loader.suiteClass((TestClass(name),))


loader = unittest.defaultTestLoader
suite = loader.suiteClass(
    [loadTestsOrSkip(loader, "test." + name) for name in testnames]
)

resultsfile = os.path.join(os.path.dirname(__file__), "cpython_tests_results.json")
if os.path.exists(resultsfile):
    os.remove(resultsfile)

runner = Runner(results_file_path=resultsfile, verbosity=2)
runner.run(suite)

print("Done! results are available in", resultsfile)
