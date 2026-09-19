import unittest
from test._test_multiprocessing import install_tests_in_module_dict

install_tests_in_module_dict(globals(), 'spawn', only_type="processes")

import sys  # TODO: RUSTPYTHON
class WithProcessesTestLock(WithProcessesTestLock):  # TODO: RUSTPYTHON
    @unittest.skipIf(  # TODO: RUSTPYTHON
        sys.platform == 'linux',  # TODO: RUSTPYTHON
        'TODO: RUSTPYTHON flaky BrokenPipeError, flaky ConnectionRefusedError, flaky ConnectionResetError, flaky EOFError'
    )  # TODO: RUSTPYTHON
    def test_repr_rlock(self): super().test_repr_rlock()  # TODO: RUSTPYTHON

if __name__ == '__main__':
    unittest.main()
