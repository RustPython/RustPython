#
# test_codecencodings_tw.py
#   Codec encoding tests for ROC encodings.
#

from test import multibytecodec_support
import unittest

class Test_Big5(multibytecodec_support.TestBase, unittest.TestCase):
    encoding = 'big5'
    tstring = multibytecodec_support.load_teststring('big5')
    codectests = (
        # invalid bytes
        (b"abc\x80\x80\xc1\xc4", "strict",  None),
        (b"abc\xc8", "strict",  None),
        (b"abc\x80\x80\xc1\xc4", "replace", "abc\ufffd\ufffd\u8b10"),
        (b"abc\x80\x80\xc1\xc4\xc8", "replace", "abc\ufffd\ufffd\u8b10\ufffd"),
        (b"abc\x80\x80\xc1\xc4", "ignore",  "abc\u8b10"),
    )

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5
    def test_callback_None_index(self):
        return super().test_callback_None_index()

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5
    def test_callback_backward_index(self):
        return super().test_callback_backward_index()

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5
    def test_callback_forward_index(self):
        return super().test_callback_forward_index()

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5
    def test_callback_index_outofbound(self):
        return super().test_callback_index_outofbound()

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5
    def test_callback_long_index(self):
        return super().test_callback_long_index()

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5
    def test_callback_returns_bytes(self):
        return super().test_callback_returns_bytes()

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5
    def test_callback_wrong_objects(self):
        return super().test_callback_wrong_objects()

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5
    def test_chunkcoding(self):
        return super().test_chunkcoding()

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5
    def test_customreplace_encode(self):
        return super().test_customreplace_encode()

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5
    def test_errorhandle(self):
        return super().test_errorhandle()

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5
    def test_incrementaldecoder(self):
        return super().test_incrementaldecoder()

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5
    def test_incrementalencoder(self):
        return super().test_incrementalencoder()

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5
    def test_incrementalencoder_del_segfault(self):
        return super().test_incrementalencoder_del_segfault()

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5
    def test_incrementalencoder_error_callback(self):
        return super().test_incrementalencoder_error_callback()

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5
    def test_null_terminator(self):
        return super().test_null_terminator()

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5
    def test_streamreader(self):
        return super().test_streamreader()

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5
    def test_streamwriter(self):
        return super().test_streamwriter()

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5
    def test_streamwriter_reset_no_pending(self):
        return super().test_streamwriter_reset_no_pending()

    @unittest.expectedFailure  # TODO: RUSTPYTHON; LookupError: unknown encoding: big5
    def test_xmlcharrefreplace(self):
        return super().test_xmlcharrefreplace()

if __name__ == "__main__":
    unittest.main()
