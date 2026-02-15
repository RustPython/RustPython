#
# test_codecmaps_cn.py
#   Codec mapping tests for PRC encodings
#

from test import multibytecodec_support
import unittest

class TestGB2312Map(multibytecodec_support.TestBase_Mapping,
                   unittest.TestCase):
    encoding = 'gb2312'
    mapfileurl = 'http://www.pythontest.net/unicode/EUC-CN.TXT'

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: gb2312
    def test_mapping_file(self):
        return super().test_mapping_file()

class TestGBKMap(multibytecodec_support.TestBase_Mapping,
                   unittest.TestCase):
    encoding = 'gbk'
    mapfileurl = 'http://www.pythontest.net/unicode/CP936.TXT'

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: gbk
    def test_mapping_file(self):
        return super().test_mapping_file()

class TestGB18030Map(multibytecodec_support.TestBase_Mapping,
                     unittest.TestCase):
    encoding = 'gb18030'
    mapfileurl = 'http://www.pythontest.net/unicode/gb-18030-2000.xml'

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: gb18030
    def test_mapping_file(self):
        return super().test_mapping_file()


if __name__ == "__main__":
    unittest.main()
