"""A failed codec lookup must report the original requested spelling."""

import codecs
import unittest
import warnings


class RegistryNormalizationTests(unittest.TestCase):
    def test_unknown_encoding_preserves_original_spelling(self):
        original = "CodecMissingÉ--A"
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", DeprecationWarning)
            with self.assertRaises(LookupError) as cm:
                codecs.lookup(original)
        self.assertEqual(str(cm.exception), f"unknown encoding: {original}")


if __name__ == "__main__":
    unittest.main()
