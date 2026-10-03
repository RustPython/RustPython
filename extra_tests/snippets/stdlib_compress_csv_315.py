"""Focused interpreter regressions for the rc2 library batch, no skips."""

import csv
import io
import itertools
import statistics
import unittest


class CompressOrderingTests(unittest.TestCase):
    def test_data_exhaustion_does_not_consume_selector(self):
        for data in ([], [3.5, 4.0, 5.25]):
            with self.subTest(data=data):
                selectors = itertools.count(1)
                self.assertEqual(list(itertools.compress(iter(data), selectors)), data)
                self.assertEqual(next(selectors), len(data) + 1)

    def test_selectors_exhaustion_consumes_one_data_item(self):
        data = iter([1, 2, 3])
        self.assertEqual(list(itertools.compress(data, [])), [])
        self.assertEqual(next(data), 2)

    def test_exhausted_data_never_evaluates_selector(self):
        class InvalidTruth:
            def __bool__(self):
                raise RuntimeError("must not evaluate selector")

        self.assertEqual(list(itertools.compress([], [InvalidTruth()])), [])

    def test_false_selectors_stop_when_data_stops(self):
        selectors = iter([False, False, False])
        self.assertEqual(list(itertools.compress([], selectors)), [])
        self.assertIs(next(selectors), False)

    def test_data_exception_precedes_selector_exception(self):
        class FailingIter:
            def __init__(self, name):
                self.name = name

            def __iter__(self):
                return self

            def __next__(self):
                raise RuntimeError(self.name)

        with self.assertRaisesRegex(RuntimeError, "^data$"):
            next(itertools.compress(FailingIter("data"), FailingIter("selector")))

    def test_rc2_fmean_nonempty_iterator(self):
        self.assertEqual(statistics.fmean(iter([3.5, 4.0, 5.25])), 4.25)

    def test_rc2_fmean_empty_iterator(self):
        with self.assertRaises(statistics.StatisticsError):
            statistics.fmean(iter([]))


class CsvDialectErrorsTests(unittest.TestCase):
    def constructors(self):
        return (
            lambda d: csv.reader([], dialect=d),
            lambda d: csv.writer(io.StringIO(), dialect=d),
        )

    def test_existing_keyword_dialect(self):
        self.assertEqual(list(csv.reader(["a\tb"], dialect="excel-tab")), [["a", "b"]])

    def test_getattr_exception(self):
        class BadDialect:
            def __getattr__(self, name):
                raise RuntimeError("boom")

        for constructor in self.constructors():
            with self.assertRaisesRegex(RuntimeError, "^boom$"):
                constructor(BadDialect())

    def test_each_dialect_attribute_exception(self):
        for name in (
            "delimiter",
            "doublequote",
            "escapechar",
            "lineterminator",
            "quotechar",
            "quoting",
            "skipinitialspace",
            "strict",
        ):
            for constructor in self.constructors():
                with self.subTest(name=name):

                    class BadDialect:
                        def __getattr__(self, attr):
                            if attr == name:
                                raise RuntimeError(name)
                            return getattr(csv.excel, attr)

                    with self.assertRaisesRegex(RuntimeError, "^" + name + "$"):
                        constructor(BadDialect())

    def test_strict_bool_exception(self):
        class InvalidTruth:
            def __bool__(self):
                raise RuntimeError("bool strict")

        class BadDialect(csv.excel):
            strict = InvalidTruth()

        for constructor in self.constructors():
            with self.assertRaisesRegex(RuntimeError, "^bool strict$"):
                constructor(BadDialect())


if __name__ == "__main__":
    unittest.main(verbosity=2)
