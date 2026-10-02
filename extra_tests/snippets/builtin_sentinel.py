# Sentinel cases copied unchanged from CPython v3.15.0rc2 Lib/test/test_builtin.py.
# https://github.com/python/cpython/blob/v3.15.0rc2/Lib/test/test_builtin.py
import copy
import gc
import pickle
import sys
import typing
import unittest
import weakref

# The snippet harness also runs CPython 3.14, which predates PEP 661.
# Always run on RustPython, even before its advertised version is upgraded.
if sys.implementation.name != "rustpython" and sys.version_info < (3, 15):
    print("Skipping sentinel tests: CPython 3.15 or newer is required")
    sys.exit(0)

from builtins import sentinel

from test import support

A_SENTINEL = sentinel("A_SENTINEL")


class SentinelContainer:
    CLASS_SENTINEL = sentinel("SentinelContainer.CLASS_SENTINEL")


class SentinelTests(unittest.TestCase):
    def test_sentinel(self):
        missing = sentinel("MISSING")
        other = sentinel("MISSING")

        self.assertIsInstance(missing, sentinel)
        self.assertIs(type(missing), sentinel)
        self.assertEqual(missing.__name__, "MISSING")
        self.assertEqual(missing.__module__, __name__)
        self.assertIsNot(missing, other)
        self.assertEqual(repr(missing), "MISSING")
        self.assertTrue(missing)
        self.assertIs(copy.copy(missing), missing)
        self.assertIs(copy.deepcopy(missing), missing)
        self.assertEqual(missing, missing)
        self.assertNotEqual(missing, other)
        self.assertRaises(TypeError, sentinel)
        self.assertRaises(TypeError, sentinel, "MISSING", "EXTRA")
        self.assertRaises(TypeError, sentinel, name="MISSING")
        with self.assertRaisesRegex(TypeError, "must be str"):
            sentinel(1)
        self.assertTrue(sentinel.__flags__ & support._TPFLAGS_IMMUTABLETYPE)
        self.assertTrue(sentinel.__flags__ & support._TPFLAGS_HAVE_GC)
        self.assertFalse(sentinel.__flags__ & support._TPFLAGS_BASETYPE)
        with self.assertRaises(TypeError):

            class SubSentinel(sentinel):
                pass

    def test_sentinel_attributes(self):
        missing = sentinel("MISSING")
        with self.assertRaises(TypeError):
            sentinel.attribute = "value"
        with self.assertRaises(AttributeError):
            missing.attribute = "value"
        with self.assertRaises(AttributeError):
            missing.__name__ = "CHANGED"
        missing.__module__ = "changed"
        self.assertEqual(missing.__module__, "changed")
        with self.assertRaises(AttributeError):
            del missing.__name__
        del missing.__module__
        with self.assertRaises(AttributeError):
            missing.__module__

    def test_sentinel_repr(self):
        with_repr = sentinel("WITH_REPR", repr="custom")
        without_repr = sentinel("WITHOUT_REPR", repr=None)
        self.assertEqual(repr(with_repr), "custom")
        self.assertEqual(repr(without_repr), "WITHOUT_REPR")
        self.assertEqual(str(with_repr), "custom")
        self.assertEqual(str(without_repr), "WITHOUT_REPR")

        with self.assertRaisesRegex(TypeError, "repr.*str or None"):
            sentinel("BAD_REPR", repr=42)

    def test_sentinel_pickle(self):
        for proto in range(pickle.HIGHEST_PROTOCOL + 1):
            with self.subTest(protocol=proto):
                self.assertIs(
                    pickle.loads(pickle.dumps(A_SENTINEL, protocol=proto)), A_SENTINEL
                )
                self.assertIs(
                    pickle.loads(
                        pickle.dumps(SentinelContainer.CLASS_SENTINEL, protocol=proto)
                    ),
                    SentinelContainer.CLASS_SENTINEL,
                )

        missing = sentinel("MISSING")
        for proto in range(pickle.HIGHEST_PROTOCOL + 1):
            with self.subTest(protocol=proto):
                with self.assertRaises(pickle.PicklingError):
                    pickle.dumps(missing, protocol=proto)

    def test_sentinel_str_subclass_name_cycle(self):
        class Name(str):
            pass

        name = Name("MISSING")
        missing = sentinel(name)
        self.assertIs(missing.__name__, name)
        self.assertTrue(gc.is_tracked(missing))

        name.missing = missing
        ref = weakref.ref(name)
        del name, missing
        support.gc_collect()
        self.assertIsNone(ref())

    def test_sentinel_union(self):
        missing = sentinel("MISSING")

        self.assertIsInstance(missing | int, typing.Union)
        self.assertEqual((missing | int).__args__, (missing, int))
        self.assertIsInstance(int | missing, typing.Union)
        self.assertEqual((int | missing).__args__, (int, missing))
        self.assertIs(missing | missing, missing)
        self.assertEqual(repr(int | missing), "int | MISSING")
        self.assertIsInstance(missing | None, typing.Union)
        self.assertEqual((missing | None).__args__, (missing, type(None)))
        self.assertIsInstance(None | missing, typing.Union)
        self.assertEqual((None | missing).__args__, (type(None), missing))
        self.assertIsInstance(missing | list[int], typing.Union)
        self.assertEqual((missing | list[int]).__args__, (missing, list[int]))
        self.assertIsInstance(missing | (int | str), typing.Union)
        self.assertEqual((missing | (int | str)).__args__, (missing, int, str))

        with self.assertRaises(TypeError):
            missing | 1
        with self.assertRaises(TypeError):
            1 | missing


class SentinelEdgeTests(unittest.TestCase):
    def test_documentation(self):
        self.assertEqual(
            sentinel.__doc__, "Create a unique sentinel object with the given name."
        )
        self.assertEqual(sentinel.__text_signature__, "(name, /, *, repr=None)")

    def test_caller_module(self):
        def factory():
            return sentinel("FROM_FUNCTION")

        factory.__module__ = "changed.module"
        self.assertEqual(factory().__module__, "changed.module")
        factory.__module__ = None
        self.assertIsNone(factory().__module__)
        for namespace, expected in (
            ({}, None),
            ({"__name__": "exec.module"}, "exec.module"),
            ({"__name__": 42}, 42),
        ):
            exec('value = sentinel("FROM_EXEC")', namespace)
            self.assertEqual(namespace["value"].__module__, expected)

    def test_module_name_snapshot(self):
        for initial in (None, "initial.module", 42):
            for change in ("__name__ = 'changed'", "del __name__"):
                namespace = {"__name__": initial}
                exec(change + "\nvalue = sentinel('VALUE')", namespace)
                self.assertEqual(namespace["value"].__module__, initial)
        namespace = {}
        exec("__name__ = 'changed'; value = sentinel('VALUE')", namespace)
        self.assertIsNone(namespace["value"].__module__)

    def test_gc_reference_cycles(self):
        class Name(str):
            pass

        name = Name("SELF_CYCLE")
        value = sentinel(name)
        ref = weakref.ref(name)
        value.__module__ = value
        del name, value
        support.gc_collect()
        self.assertIsNone(ref())

        first_name = Name("FIRST")
        second_name = Name("SECOND")
        first = sentinel(first_name)
        second = sentinel(second_name)
        first_ref = weakref.ref(first_name)
        second_ref = weakref.ref(second_name)
        first.__module__ = second
        second.__module__ = first
        del first_name, second_name, first, second
        support.gc_collect()
        self.assertIsNone(first_ref())
        self.assertIsNone(second_ref())

        custom_repr = Name("CUSTOM_REPR")
        value = sentinel("REPR_CYCLE", repr=custom_repr)
        custom_repr.value = value
        ref = weakref.ref(custom_repr)
        del custom_repr, value
        support.gc_collect()
        self.assertIsNone(ref())

    def test_string_subclasses_and_unicode(self):
        class Name(str):
            pass

        for text in ("", "MISSING", "not an identifier", "\ud800"):
            name = Name(text)
            value = sentinel(name)
            self.assertIs(value.__name__, name)
            self.assertEqual(repr(value), text)
            self.assertIs(value.__reduce__(), name)
            custom = Name("custom " + text)
            self.assertEqual(repr(sentinel(name, repr=custom)), custom)

    def test_identity_and_protocols(self):
        value = sentinel("VALUE")
        other = sentinel("VALUE")
        self.assertEqual(len({value, other}), 2)
        self.assertTrue(value)
        self.assertEqual(value.__reduce_ex__(5), "VALUE")
        with self.assertRaises(TypeError):
            value < other
        with self.assertRaises(TypeError):
            weakref.ref(value)
        with self.assertRaises(TypeError):
            sentinel("VALUE", bool=False)
        with self.assertRaises(TypeError):
            sentinel("VALUE", "repr")
        with self.assertRaises(TypeError):
            sentinel("VALUE", module="elsewhere")
        with self.assertRaises(TypeError):
            value.__class__ = object
        value.__module__ = object()
        del value.__module__
        with self.assertRaises(AttributeError):
            del value.__module__


if __name__ == "__main__":
    unittest.main()
