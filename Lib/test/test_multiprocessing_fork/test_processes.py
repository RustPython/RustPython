import unittest
from test._test_multiprocessing import install_tests_in_module_dict

install_tests_in_module_dict(globals(), 'fork', only_type="processes")

class WithProcessesTestPicklingConnections(WithProcessesTestPicklingConnections):  # TODO: RUSTPYTHON
    @unittest.skip('TODO: RUSTPYTHON; hangs')
    def test_pickling(self): super().test_pickling()

if __name__ == '__main__':
    unittest.main()
