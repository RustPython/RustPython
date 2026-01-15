import unittest
from test._test_multiprocessing import install_tests_in_module_dict

install_tests_in_module_dict(globals(), 'spawn', only_type="processes")

import os, sys  # TODO: RUSTPYTHON
class WithProcessesTestCondition(WithProcessesTestCondition):  # TODO: RUSTPYTHON
    @unittest.skipIf(sys.platform == 'darwin', 'TODO: RUSTPYTHON flaky timeout')
    def test_notify(self): super().test_notify()

class WithProcessesTestLock(WithProcessesTestLock):  # TODO: RUSTPYTHON
    @unittest.skipIf(  # TODO: RUSTPYTHON
        sys.platform in ('darwin', 'linux') and 'RUSTPYTHON_SKIP_ENV_POLLUTERS' in os.environ,  # TODO: RUSTPYTHON
        'TODO: RUSTPYTHON environment pollution when running rustpython -m test --fail-env-changed due to unknown reason'
    )  # TODO: RUSTPYTHON
    def test_repr_lock(self): super().test_repr_lock()  # TODO: RUSTPYTHON
    @unittest.skipIf(  # TODO: RUSTPYTHON
        sys.platform == 'linux',  # TODO: RUSTPYTHON
        'TODO: RUSTPYTHON flaky BrokenPipeError, flaky ConnectionRefusedError, flaky ConnectionResetError, flaky EOFError'
    )  # TODO: RUSTPYTHON
    def test_repr_rlock(self): super().test_repr_rlock()  # TODO: RUSTPYTHON

class WithProcessesTestPool(WithProcessesTestPool):  # TODO: RUSTPYTHON
    @unittest.skipIf(  # TODO: RUSTPYTHON
        sys.platform in ('darwin', 'linux') and 'RUSTPYTHON_SKIP_ENV_POLLUTERS' in os.environ,  # TODO: RUSTPYTHON
        'TODO: RUSTPYTHON environment pollution when running rustpython -m test --fail-env-changed due to unknown reason'
    )  # TODO: RUSTPYTHON
    def test_async_timeout(self): super().test_async_timeout()  # TODO: RUSTPYTHON
    @unittest.skipIf(  # TODO: RUSTPYTHON
        sys.platform in ('darwin', 'linux') and 'RUSTPYTHON_SKIP_ENV_POLLUTERS' in os.environ,  # TODO: RUSTPYTHON
        'TODO: RUSTPYTHON environment pollution when running rustpython -m test --fail-env-changed due to unknown reason'
    )  # TODO: RUSTPYTHON
    def test_terminate(self): super().test_terminate()  # TODO: RUSTPYTHON
    @unittest.skipIf(  # TODO: RUSTPYTHON
        sys.platform in ('darwin', 'linux') and 'RUSTPYTHON_SKIP_ENV_POLLUTERS' in os.environ,  # TODO: RUSTPYTHON
        'TODO: RUSTPYTHON environment pollution when running rustpython -m test --fail-env-changed due to unknown reason'
    )  # TODO: RUSTPYTHON
    def test_traceback(self): super().test_traceback()  # TODO: RUSTPYTHON

if __name__ == '__main__':
    unittest.main()
