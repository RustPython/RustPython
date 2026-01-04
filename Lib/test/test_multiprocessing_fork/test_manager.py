import unittest
from test._test_multiprocessing import install_tests_in_module_dict

install_tests_in_module_dict(globals(), 'fork', only_type="manager")

import sys  # TODO: RUSTPYTHON
class WithManagerTestCondition(WithManagerTestCondition):  # TODO: RUSTPYTHON
    @unittest.skipIf(sys.platform == 'linux', 'TODO: RUSTPYTHON, times out')
    def test_notify_all(self): super().test_notify_all()  # TODO: RUSTPYTHON

class WithManagerTestQueue(WithManagerTestQueue):  # TODO: RUSTPYTHON
    @unittest.skipIf(sys.platform == 'linux', 'TODO: RUSTPYTHON, times out')
    def test_fork(self): super().test_fork()  # TODO: RUSTPYTHON

local_globs = globals().copy()  # TODO: RUSTPYTHON
for name, base in local_globs.items():  # TODO: RUSTPYTHON
    if name.startswith('WithManagerTest') and issubclass(base, unittest.TestCase):  # TODO: RUSTPYTHON
        base = unittest.skipIf(  # TODO: RUSTPYTHON
            sys.platform == 'linux',  # TODO: RUSTPYTHON
            'TODO: RUSTPYTHON flaky BrokenPipeError, flaky ConnectionRefusedError, flaky ConnectionResetError, flaky EOFError'
        )(base)  # TODO: RUSTPYTHON

if __name__ == '__main__':
    unittest.main()
