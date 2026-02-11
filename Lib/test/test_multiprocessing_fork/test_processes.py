import unittest
from test._test_multiprocessing import install_tests_in_module_dict

install_tests_in_module_dict(globals(), 'fork', only_type="processes")

import os, sys  # TODO: RUSTPYTHON
class WithProcessesTestCondition(WithProcessesTestCondition):  # TODO: RUSTPYTHON
    @unittest.skipIf(sys.platform == 'linux', 'TODO: RUSTPYTHON flaky timeout')
    def test_notify_all(self): super().test_notify_all()  # TODO: RUSTPYTHON

class WithProcessesTestLock(WithProcessesTestLock):  # TODO: RUSTPYTHON
    @unittest.skipIf(sys.platform == 'linux', 'TODO: RUSTPYTHON flaky BrokenPipeError, flaky ConnectionRefusedError, flaky ConnectionResetError, flaky EOFError')
    def test_repr_lock(self): super().test_repr_lock()  # TODO: RUSTPYTHON

class WithProcessesTestManagerRestart(WithProcessesTestManagerRestart):  # TODO: RUSTPYTHON
    @unittest.skipIf(sys.platform == 'linux', 'TODO: RUSTPYTHON flaky BrokenPipeError, flaky ConnectionRefusedError, flaky ConnectionResetError, flaky EOFError')
    def test_rapid_restart(self): super().test_rapid_restart()  # TODO: RUSTPYTHON

class WithProcessesTestProcess(WithProcessesTestProcess):  # TODO: RUSTPYTHON
    @unittest.skipIf(sys.platform == 'linux', 'TODO: RUSTPYTHON flaky timeout')
    def test_args_argument(self): super().test_args_argument()  # TODO: RUSTPYTHON
    @unittest.skipIf(sys.platform == 'linux', 'TODO: RUSTPYTHON flaky timeout')
    def test_process(self): super().test_process()  # TODO: RUSTPYTHON

class WithProcessesTestPoolWorkerLifetime(WithProcessesTestPoolWorkerLifetime):  # TODO: RUSTPYTHON
    @unittest.skipIf(sys.platform == 'linux', 'TODO: RUSTPYTHON flaky timeout')
    def test_pool_worker_lifetime(self): super().test_pool_worker_lifetime()  # TODO: RUSTPYTHON
    @unittest.skipIf(sys.platform == 'linux', 'TODO: RUSTPYTHON flaky timeout')
    def test_pool_worker_lifetime_early_close(self): super().test_pool_worker_lifetime_early_close()  # TODO: RUSTPYTHON

class WithProcessesTestQueue(WithProcessesTestQueue):  # TODO: RUSTPYTHON
    @unittest.skipIf(sys.platform == 'linux', 'TODO: RUSTPYTHON flaky timeout')
    def test_fork(self): super().test_fork()  # TODO: RUSTPYTHON
    @unittest.skipIf(sys.platform == 'linux', 'TODO: RUSTPYTHON flaky timeout')
    def test_get(self): super().test_get()  # TODO: RUSTPYTHON

class WithProcessesTestSharedMemory(WithProcessesTestSharedMemory):  # TODO: RUSTPYTHON
    @unittest.skipIf(sys.platform == 'linux', 'TODO: RUSTPYTHON flaky BrokenPipeError, flaky ConnectionRefusedError, flaky ConnectionResetError, flaky EOFError')
    def test_shared_memory_SharedMemoryManager_basics(self): super().test_shared_memory_SharedMemoryManager_basics()  # TODO: RUSTPYTHON

if __name__ == '__main__':
    unittest.main()
