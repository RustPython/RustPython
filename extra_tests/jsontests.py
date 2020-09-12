import unittest
from custom_text_test_runner import CustomTextTestRunner as Runner
from test.libregrtest.runtest import findtests
import os

testnames = ('test.' + name for name in findtests())

suite = unittest.defaultTestLoader.loadTestsFromNames(testnames)

resultsfile = os.path.join(os.path.dirname(__file__), "cpython_tests_results.json")
if os.path.exists(resultsfile):
    os.remove(resultsfile)

runner = Runner(results_file_path=resultsfile)
runner.run(suite)

print("Done! results are available in", resultsfile)
