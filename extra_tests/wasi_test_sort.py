"""Include test_sort's legacy stress-error counter in the WASI exit status."""

import sys
import unittest

from test import test_sort

result = unittest.main(module=test_sort, exit=False).result
if test_sort.nerrors:
    print(f"Sort stress mismatches: {test_sort.nerrors}", file=sys.stderr)
if not result.wasSuccessful() or test_sort.nerrors:
    sys.exit(1)
if result.testsRun == 0:
    sys.exit(5)
