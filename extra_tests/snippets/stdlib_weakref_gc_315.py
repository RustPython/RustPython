"""Regressions for callbacks created or retained during cyclic collection."""

import gc
import unittest
import weakref


class WeakrefGcOrderingTests(unittest.TestCase):
    def setUp(self):
        self.was_enabled = gc.isenabled()
        gc.disable()
        gc.collect()

    def tearDown(self):
        gc.collect()
        if self.was_enabled:
            gc.enable()

    def test_new_callback_in_finalizer_is_not_called(self):
        refs = []
        callbacks = []

        class Class:
            def __del__(self):
                refs.append(weakref.ref(self, lambda wr: callbacks.append("called")))

        obj = Class()
        obj.cycle = obj
        del obj
        gc.collect()
        self.assertEqual(len(refs), 1)
        self.assertIsNone(refs[0]())
        self.assertEqual(callbacks, [])

    def test_gc_keeps_callback_alive_until_weakref_dies(self):
        calls = []

        class Class:
            pass

        class Callback:
            def __call__(self, wr):
                calls.append(wr() is None)

        obj = Class()
        obj.cycle = obj
        callback = Callback()
        callback_observer = weakref.ref(callback)
        wr = weakref.ref(obj, callback)
        del callback, obj
        gc.collect()
        self.assertIsNone(wr())
        self.assertIs(wr.__callback__, callback_observer())
        self.assertEqual(calls, [True])
        self.assertIsNotNone(callback_observer())
        del wr
        self.assertIsNone(callback_observer())


if __name__ == "__main__":
    unittest.main(verbosity=2)
