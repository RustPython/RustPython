"""Preserve source parameter order when evaluating function annotations."""

import functools
import typing
import unittest


class AnnotationOrderTests(unittest.TestCase):
    def test_deferred_order_and_evaluation(self):
        seen = []

        def annotate(name):
            seen.append(name)
            return int

        namespace = {"annotate": annotate}
        exec(
            'def f(pos: annotate("pos"), /, normal: annotate("normal"), '
            '*args: annotate("args"), kw: annotate("kw"), '
            '**kwargs: annotate("kwargs")) -> annotate("return"): pass',
            namespace,
        )
        f = namespace["f"]
        self.assertEqual(seen, [])
        expected = ["pos", "normal", "args", "kw", "kwargs", "return"]
        self.assertEqual(list(f.__annotations__), expected)
        self.assertEqual(seen, expected)
        self.assertEqual(list(typing.get_type_hints(f)), expected)

    def test_future_order(self):
        namespace = {}
        exec(
            "from __future__ import annotations\n"
            "def f(pos: int, /, normal: str, *args: int, "
            "kw: bool, **kwargs: str) -> None: pass",
            namespace,
        )
        self.assertEqual(
            list(namespace["f"].__annotations__),
            ["pos", "normal", "args", "kw", "kwargs", "return"],
        )

    def test_singledispatch_positional_only(self):
        @functools.singledispatch
        def choose(arg, /, extra):
            return "base"

        @choose.register
        def _(arg: int, /, extra: str):
            return "int"

        @choose.register
        def _(arg: str, /, extra: int):
            return "str"

        self.assertEqual(choose(1, ""), "int")
        self.assertEqual(choose("", 1), "str")


if __name__ == "__main__":
    unittest.main()
