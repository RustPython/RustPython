import unittest
from test._test_multiprocessing import install_tests_in_module_dict

install_tests_in_module_dict(globals(), 'forkserver', only_type="threads")

import os  # TODO: RUSTPYTHON
class WithThreadsTestPool(WithThreadsTestPool):  # TODO: RUSTPYTHON
    @unittest.skipIf(  # TODO: RUSTPYTHON
        'RUSTPYTHON_SKIP_ENV_POLLUTERS' in os.environ,  # TODO: RUSTPYTHON
        'TODO: RUSTPYTHON environment pollution when running rustpython -m test --fail-env-changed due to unknown reason'
    )  # TODO: RUSTPYTHON
    def test_terminate(self): super().test_terminate()  # TODO: RUSTPYTHON

if __name__ == '__main__':
    unittest.main()
