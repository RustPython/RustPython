"""Exercise the keyword binding used by rc2 base64.b16encode wrapping."""

import base64
import binascii
import unittest


class HexlifyKeywordTests(unittest.TestCase):
    def test_keyword_and_positional_options(self):
        for encode in (binascii.hexlify, binascii.b2a_hex):
            for count in (-3, -2, -1, 0, 1, 2, 3):
                with self.subTest(encode=encode.__name__, count=count):
                    expected = b"abcde".hex("-", count).encode("ascii")
                    self.assertEqual(
                        encode(b"abcde", sep="-", bytes_per_sep=count), expected
                    )
                    self.assertEqual(
                        encode(b"abcde", b"-", bytes_per_sep=count), expected
                    )
                    self.assertEqual(encode(b"abcde", b"-", count), expected)
            self.assertEqual(encode(b"abc", bytes_per_sep=-2), b"616263")
            self.assertEqual(encode(data=b"abc"), b"616263")
            with self.assertRaises(TypeError):
                encode(b"abc", b"-", sep=b"-")

    def test_base16_wrapping(self):
        self.assertEqual(base64.b16encode(b"abcde", wrapcol=5), b"6162\n6364\n65")
        self.assertEqual(base64.b16encode(b"abcde", wrapcol=4), b"6162\n6364\n65")


if __name__ == "__main__":
    unittest.main()
