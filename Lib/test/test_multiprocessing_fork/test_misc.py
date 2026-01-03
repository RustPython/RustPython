import unittest
from test._test_multiprocessing import install_tests_in_module_dict

install_tests_in_module_dict(globals(), 'fork', exclude_types=True)

import sys  # TODO: RUSTPYTHON
class TestManagerExceptions(TestManagerExceptions):  # TODO: RUSTPYTHON
    @unittest.skipIf(sys.platform == 'linux', "TODO: RUSTPYTHON flaky")
    def test_queue_get(self): super().test_queue_get()  # TODO: RUSTPYTHON

@unittest.skipIf(sys.platform == 'linux', "TODO: RUSTPYTHON flaky")
class TestInitializers(TestInitializers): pass  # TODO: RUSTPYTHON

class TestStartMethod(TestStartMethod):  # TODO: RUSTPYTHON
    @unittest.skipIf(sys.platform == 'linux', "TODO: RUSTPYTHON flaky")
    def test_nested_startmethod(self): super().test_nested_startmethod()  # TODO: RUSTPYTHON

@unittest.skipIf(sys.platform == 'linux', "TODO: RUSTPYTHON flaky")
class TestSyncManagerTypes(TestSyncManagerTypes): pass  # TODO: RUSTPYTHON

class MiscTestCase(MiscTestCase):  # TODO: RUSTPYTHON
    @unittest.skipIf(sys.platform == 'linux', "TODO: RUSTPYTHON flaky")
    def test_forked_thread_not_started(self): super().test_forked_thread_not_started()  # TODO: RUSTPYTHON

if __name__ == '__main__':
    unittest.main()
