import unittest
from test._test_multiprocessing import install_tests_in_module_dict

install_tests_in_module_dict(globals(), 'fork', only_type="threads")

import os, sys  # TODO: RUSTPYTHON
class WithThreadsTestPool(WithThreadsTestPool):  # TODO: RUSTPYTHON
    @unittest.skip("TODO: RUSTPYTHON; flaky environment pollution when running rustpython -m test --fail-env-changed due to unknown reason")
    def test_terminate(self): super().test_terminate()  # TODO: RUSTPYTHON

class WithThreadsTestManagerRestart(WithThreadsTestManagerRestart):  # TODO: RUSTPYTHON
    @unittest.skipIf(sys.platform == 'linux', 'TODO: RUSTPYTHON flaky flaky BrokenPipeError, flaky ConnectionRefusedError, flaky ConnectionResetError, flaky EOFError')
    def test_rapid_restart(self): super().test_rapid_restart()  # TODO: RUSTPYTHON

if __name__ == '__main__':
    unittest.main()
