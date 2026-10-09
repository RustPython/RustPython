"""Ordered-view representation and pprint paths unique to native ordered payloads."""

import pprint
import unittest
from collections import OrderedDict


class OrderedViewExtendedTests(unittest.TestCase):
    def test_repr_keeps_surrogates_from_custom_repr(self):
        for text in ("\ud800",):

            class Element:
                def __repr__(self):
                    return text

            element = Element()
            data = OrderedDict([(element, element)])
            self.assertEqual(repr(data.keys()), "odict_keys([" + text + "])")
            self.assertEqual(repr(data.values()), "odict_values([" + text + "])")
            self.assertEqual(
                repr(data.items()), "odict_items([(" + text + ", " + text + ")])"
            )

    def test_recursive_key_repr(self):
        class Key:
            def __repr__(self):
                return repr(view)

        data = OrderedDict([(Key(), 1)])
        view = data.keys()
        self.assertEqual(repr(view), "odict_keys([...])")

    def test_repr_exception_releases_recursion_guard(self):
        class Element:
            broken = True

            def __repr__(self):
                if self.broken:
                    raise RuntimeError("element repr")
                return "element"

        element = Element()
        data = OrderedDict(a=element)
        view = data.values()
        with self.assertRaises(RuntimeError):
            repr(view)
        element.broken = False
        self.assertEqual(repr(view), "odict_values([element])")

    def test_mutating_first_repr_keeps_later_snapshot_entries(self):
        calls = []

        class Element:
            def __init__(self, name):
                self.name = name

            def __repr__(self):
                calls.append(self.name)
                data.clear()
                return self.name

        data = OrderedDict(a=Element("first"), b=Element("second"))
        self.assertEqual(repr(data.values()), "odict_values([first, second])")
        self.assertEqual(calls, ["first", "second"])
        self.assertFalse(data)

    def test_pprint_mixed_items_sort_by_safe_tuple(self):
        data = OrderedDict([("b", 2), (1, "one")])
        self.assertEqual(
            pprint.pformat(data.items()), "odict_items([(1, 'one'), ('b', 2)])"
        )
        rendered = pprint.pformat(data.items(), width=12)
        self.assertTrue(rendered.startswith("odict_items([(1,"), rendered)

    def test_pprint_recursion_flags(self):
        for method in ("values", "items"):
            data = OrderedDict(a=1)
            view = getattr(data, method)()
            data["v"] = view
            self.assertTrue(pprint.isrecursive(view))
            self.assertFalse(pprint.isreadable(view))
            self.assertIn("Recursion on odict_" + method, pprint.pformat(view))


if __name__ == "__main__":
    unittest.main(verbosity=2)
