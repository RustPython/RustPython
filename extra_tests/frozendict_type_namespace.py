"""Type-construction regressions characterized against CPython 3.15.0rc2."""

import types
import unittest


class FrozenTypeNamespaceTests(unittest.TestCase):
    def test_exact_frozen_namespace_is_copied(self):
        namespace = frozendict(value=17, __qualname__="Outer.FrozenNamespace")
        original_hash = hash(namespace)
        cls = type("FrozenNamespace", (), namespace)
        self.assertEqual(cls.value, 17)
        self.assertEqual(cls.__qualname__, "Outer.FrozenNamespace")
        cls.value = 19
        cls.added = 23
        del cls.value
        self.assertEqual(
            namespace,
            {"value": 17, "__qualname__": "Outer.FrozenNamespace"},
        )
        self.assertEqual(hash(namespace), original_hash)
        self.assertNotIn("__module__", namespace)
        self.assertNotIn("added", namespace)

    def test_builtin_iterator_bypasses_subclass_mapping_overrides(self):
        for base in (dict, frozendict):

            class Namespace(base):
                def keys(self):
                    raise AssertionError("keys must not be called")

                def __getitem__(self, key):
                    raise AssertionError("__getitem__ must not be called")

                def __len__(self):
                    raise AssertionError("__len__ must not be called")

            with self.subTest(base=base):
                namespace = Namespace(value=17, __qualname__="Outer.C")
                cls = type("C", (), namespace)
                self.assertEqual(cls.value, 17)
                self.assertEqual(cls.__qualname__, "Outer.C")
                self.assertEqual(base.__getitem__(namespace, "value"), 17)

    def test_custom_iterator_uses_mapping_order_and_values(self):
        for base in (dict, frozendict):
            calls = []

            class Namespace(base):
                def __iter__(self):
                    raise AssertionError("the copy must use keys, not __iter__")

                def keys(self):
                    calls.append("keys")
                    return ["second", "first"]

                def __getitem__(self, key):
                    calls.append(key)
                    return {"first": 11, "second": 22}[key]

            with self.subTest(base=base):
                namespace = Namespace(stored=3)
                cls = type("C", (), namespace)
                self.assertEqual(calls, ["keys", "second", "first"])
                self.assertEqual((cls.first, cls.second), (11, 22))
                self.assertFalse(hasattr(cls, "stored"))
                self.assertEqual(
                    [key for key in cls.__dict__ if not key.startswith("__")],
                    ["second", "first"],
                )
                self.assertEqual(base.__getitem__(namespace, "stored"), 3)

    def test_empty_subclass_copy_bypasses_mapping_overrides(self):
        for base in (dict, frozendict):

            class Namespace(base):
                def __iter__(self):
                    raise AssertionError("__iter__ must not be called")

                def keys(self):
                    raise AssertionError("keys must not be called")

                def __getitem__(self, key):
                    raise AssertionError("__getitem__ must not be called")

                def __len__(self):
                    return 1

            with self.subTest(base=base):
                namespace = Namespace()
                cls = type("C", (), namespace)
                self.assertEqual(cls.__name__, "C")
                self.assertEqual(base.__len__(namespace), 0)

    def test_class_cells_use_a_mutable_copy(self):
        classcell = types.CellType()
        dictcell = types.CellType()
        namespace = frozendict(
            value=17,
            __classcell__=classcell,
            __classdictcell__=dictcell,
        )
        cls = type("C", (), namespace)
        self.assertIs(classcell.cell_contents, cls)
        classdict = dictcell.cell_contents
        self.assertIs(type(classdict), dict)
        self.assertIsNot(classdict, namespace)
        self.assertNotIn("__classcell__", classdict)
        self.assertNotIn("__classdictcell__", classdict)
        self.assertIs(namespace["__classcell__"], classcell)
        self.assertIs(namespace["__classdictcell__"], dictcell)
        cls.value = 19
        self.assertEqual(classdict["value"], 19)
        self.assertEqual(namespace["value"], 17)

    def test_descriptor_initialization_does_not_change_source(self):
        calls = []

        class Descriptor:
            def __set_name__(self, owner, name):
                calls.append((owner, name))
                owner.added = 23

        descriptor = Descriptor()
        namespace = frozendict(field=descriptor)
        cls = type("C", (), namespace)
        self.assertEqual(calls, [(cls, "field")])
        self.assertEqual(cls.added, 23)
        self.assertIs(namespace["field"], descriptor)
        self.assertEqual(list(namespace), ["field"])

    def test_winning_metaclass_receives_original_namespace(self):
        for base in (dict, frozendict):

            class Namespace(base):
                def __iter__(self):
                    raise AssertionError("namespace must not be copied")

                def keys(self):
                    raise AssertionError("namespace must not be copied")

            namespace = Namespace(value=17)

            class Meta(type):
                def __new__(metacls, name, bases, ns):
                    if name == "Child":
                        return ns
                    return super().__new__(metacls, name, bases, ns)

            class Base(metaclass=Meta):
                pass

            with self.subTest(base=base):
                self.assertIs(type("Child", (Base,), namespace), namespace)

    def test_keywords_and_explicit_type_new(self):
        calls = []

        class Base:
            def __init_subclass__(cls, *, marker):
                calls.append((cls, marker))

        namespace = frozendict(value=17)
        cls = type("C", (Base,), namespace, marker=23)
        explicit = type.__new__(type, "D", (Base,), namespace, marker=29)
        self.assertEqual(calls, [(cls, 23), (explicit, 29)])
        self.assertEqual((cls.value, explicit.value), (17, 17))
        self.assertEqual(namespace, {"value": 17})

    def test_non_dictionary_mappings_remain_rejected(self):
        class Mapping:
            def keys(self):
                return ["value"]

            def __getitem__(self, key):
                return 17

        for namespace in (types.MappingProxyType({}), Mapping(), [], ()):
            with self.subTest(namespace=namespace), self.assertRaises(TypeError):
                type("C", (), namespace)


if __name__ == "__main__":
    unittest.main()
