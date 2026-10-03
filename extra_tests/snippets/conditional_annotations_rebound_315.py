"""Rebound annotation tracking must raise instead of entering an unsafe SET_ADD."""

import unittest


class SetSubclass(set):
    pass


class ConditionalAnnotationTests(unittest.TestCase):
    def test_rebound_tracking_rejects_every_nonexact_set(self):
        values = [0, {}, [], "", object(), frozenset(), SetSubclass(), None]
        for left in (
            "__conditional_annotations__",
            'globals()["__conditional_annotations__"]',
        ):
            for value in values:
                with self.subTest(left=left, value_type=type(value).__name__):
                    namespace = {"replacement": value}
                    with self.assertRaisesRegex(TypeError, "object is not a set"):
                        exec(f"{left} = replacement\nx: int\n", namespace)

    def test_exact_set_and_comprehension_still_work(self):
        replacement = set()
        namespace = {"replacement": replacement}
        exec("__conditional_annotations__ = replacement\nx: int\n", namespace)
        self.assertIs(namespace["__conditional_annotations__"], replacement)
        self.assertEqual(len(replacement), 1)
        self.assertEqual({x * x for x in range(4)}, {0, 1, 4, 9})


if __name__ == "__main__":
    unittest.main()
