"""The new Pattern/Scanner prefixmatch name keeps legacy match semantics."""

import re
import unittest


class PrefixMatchTests(unittest.TestCase):
    def test_pattern(self):
        for pattern, text in [("a+", "aab"), (b"a+", b"aab")]:
            compiled = re.compile(pattern)
            for pos, endpos in [(0, 3), (1, 2), (2, 3), (0, 0)]:
                old = compiled.match(text, pos, endpos)
                new = compiled.prefixmatch(text, pos, endpos)
                self.assertEqual(
                    None if old is None else old.span(),
                    None if new is None else new.span(),
                )
            self.assertEqual(compiled.prefixmatch(text, pos=1, endpos=2).span(), (1, 2))

    def test_scanner(self):
        for pattern in [".", "x*"]:
            compiled = re.compile(pattern)
            left = compiled.scanner("xyz")
            right = compiled.scanner("xyz")
            for _ in range(5):
                old, new = left.match(), right.prefixmatch()
                self.assertEqual(
                    None if old is None else old.span(),
                    None if new is None else new.span(),
                )


if __name__ == "__main__":
    unittest.main()
