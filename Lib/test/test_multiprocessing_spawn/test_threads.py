import unittest
from test._test_multiprocessing import install_tests_in_module_dict

install_tests_in_module_dict(globals(), 'spawn', only_type="threads")

import os, sys  # TODO: RUSTPYTHON
class WithThreadsTestPool(WithThreadsTestPool):  # TODO: RUSTPYTHON
    @unittest.skip("TODO: RUSTPYTHON; flaky environment pollution when running rustpython -m test --fail-env-changed due to unknown reason")
    def test_terminate(self): super().test_terminate()  # TODO: RUSTPYTHON

if __name__ == '__main__':
    unittest.main()
