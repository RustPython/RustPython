"""CSV strict attribute and truth-conversion exceptions must propagate."""

import csv
import unittest


class CsvDialectErrorsTests(unittest.TestCase):
    def test_strict_getattr_exception(self):
        class BadDialect:
            def __getattr__(self, name):
                if name == "strict":
                    raise RuntimeError("strict")
                return getattr(csv.excel, name)

        with self.assertRaisesRegex(RuntimeError, "^strict$"):
            csv.reader([], dialect=BadDialect())

    def test_strict_bool_exception(self):
        class InvalidTruth:
            def __bool__(self):
                raise RuntimeError("bool strict")

        class BadDialect(csv.excel):
            strict = InvalidTruth()

        with self.assertRaisesRegex(RuntimeError, "^bool strict$"):
            csv.reader([], dialect=BadDialect())


if __name__ == "__main__":
    unittest.main()
