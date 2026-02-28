import unittest
from test._test_multiprocessing import install_tests_in_module_dict

install_tests_in_module_dict(globals(), 'fork', only_type="processes")

import sys  # TODO: RUSTPYTHON
class WithProcessesTestManagerRestart(WithProcessesTestManagerRestart):  # TODO: RUSTPYTHON
    @unittest.skipIf(sys.platform == 'linux', 'TODO: RUSTPYTHON flaky segfault in Manager accepter thread after fork')
    def test_rapid_restart(self): super().test_rapid_restart()  # TODO: RUSTPYTHON

class WithProcessesTestSharedMemory(WithProcessesTestSharedMemory):  # TODO: RUSTPYTHON
    @unittest.skipIf(sys.platform == 'linux', 'TODO: RUSTPYTHON flaky segfault in Manager accepter thread after fork')
    def test_shared_memory_SharedMemoryManager_basics(self): super().test_shared_memory_SharedMemoryManager_basics()  # TODO: RUSTPYTHON

if __name__ == '__main__':
    unittest.main()
