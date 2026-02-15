#
# test_codecmaps_hk.py
#   Codec mapping tests for HongKong encodings
#

from test import multibytecodec_support
import unittest

class TestBig5HKSCSMap(multibytecodec_support.TestBase_Mapping,
                       unittest.TestCase):
    encoding = 'big5hkscs'
    mapfileurl = 'http://www.pythontest.net/unicode/BIG5HKSCS-2004.TXT'

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5hkscs
    def test_mapping_file(self):
        return super().test_mapping_file()

if __name__ == "__main__":
    unittest.main()
