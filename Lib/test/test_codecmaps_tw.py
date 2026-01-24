#
# test_codecmaps_tw.py
#   Codec mapping tests for ROC encodings
#

from test import multibytecodec_support
import unittest

@unittest.skip("TODO: RUSTPYTHON; LookupError: unknown encoding: big5")
class TestBIG5Map(multibytecodec_support.TestBase_Mapping,
                  unittest.TestCase):
    encoding = 'big5'
    mapfileurl = 'http://www.pythontest.net/unicode/BIG5.TXT'

@unittest.skip("TODO: RUSTPYTHON; LookupError: unknown encoding: cp950")
class TestCP950Map(multibytecodec_support.TestBase_Mapping,
                   unittest.TestCase):
    encoding = 'cp950'
    mapfileurl = 'http://www.pythontest.net/unicode/CP950.TXT'
    pass_enctest = [
        (b'\xa2\xcc', '\u5341'),
        (b'\xa2\xce', '\u5345'),
    ]
    codectests = (
        (b"\xFFxy", "replace",  "\ufffdxy"),
    )

if __name__ == "__main__":
    unittest.main()
