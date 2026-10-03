"""OrderedDict view repr must match dict views, including recursive values."""

import pprint
import unittest
from collections import OrderedDict


class OrderedViewReprTests(unittest.TestCase):
    def test_views_preserve_order(self):
        d = OrderedDict([("b", 2), ("a", 1)])
        self.assertEqual(repr(d.keys()), "odict_keys(['b', 'a'])")
        self.assertEqual(repr(d.values()), "odict_values([2, 1])")
        self.assertEqual(repr(d.items()), "odict_items([('b', 2), ('a', 1)])")
        d.move_to_end("b")
        self.assertEqual(repr(d.keys()), "odict_keys(['a', 'b'])")

    def test_recursive_view(self):
        for method, expected in (
            ("values", "odict_values([1, ...])"),
            ("items", "odict_items([('a', 1), ('v', ...)])"),
        ):
            d = OrderedDict(a=1)
            view = getattr(d, method)()
            d["v"] = view
            self.assertEqual(repr(view), expected)

    def test_snapshot_before_element_repr(self):
        for method in ("keys", "values", "items"):
            d = OrderedDict()

            class MutatingRepr:
                def __repr__(self):
                    d.clear()
                    return "element"

            key = MutatingRepr()
            d[key] = key
            expected = {
                "keys": "odict_keys([element])",
                "values": "odict_values([element])",
                "items": "odict_items([(element, element)])",
            }
            self.assertEqual(repr(getattr(d, method)()), expected[method])
            self.assertEqual(len(d), 0)

    def test_custom_repr_error(self):
        class Broken:
            def __repr__(self):
                raise ValueError("view repr")

        d = OrderedDict(a=Broken())
        with self.assertRaisesRegex(ValueError, "view repr"):
            repr(d.values())

    def test_pprint_dispatch(self):
        d = OrderedDict(a=1, b=2)
        self.assertEqual(pprint.pformat(d.keys(), width=12), "odict_keys(['a',\n 'b'])")


if __name__ == "__main__":
    unittest.main()
