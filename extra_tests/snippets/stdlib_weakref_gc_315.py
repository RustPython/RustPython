"""GC ordering regression probes; all assertions remain active."""

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

    def test_finalizer_observes_callback_free_ref(self):
        results = []

        def create_cycle():
            class Class:
                def __init__(self):
                    self.cycle = self
                    self.with_callback = weakref.ref(Class, lambda wr: None)
                    self.without_callback = weakref.ref(Class)

                def __del__(self):
                    results.append(
                        (self.with_callback() is None, self.without_callback() is Class)
                    )

            Class()

        create_cycle()
        gc.collect()
        self.assertEqual(results, [(True, True)])

    def test_resurrected_object_preserves_callback_free_ref(self):
        resurrected = []
        callbacks = []

        class Class:
            def __del__(self):
                resurrected.append(self)

        obj = Class()
        obj.cycle = obj
        plain = weakref.ref(obj)
        callback = weakref.ref(obj, lambda wr: callbacks.append(wr() is None))
        del obj
        gc.collect()
        self.assertEqual(len(resurrected), 1)
        self.assertIs(plain(), resurrected[0])
        self.assertIsNone(callback())
        self.assertEqual(callbacks, [True])
        resurrected.clear()
        gc.collect()
        self.assertIsNone(plain())
        self.assertEqual(callbacks, [True])

    def test_callback_free_refs_dead_after_collection(self):
        class Class:
            pass

        obj = Class()
        obj.cycle = obj
        refs = [weakref.ref(obj), weakref.proxy(obj)]
        del obj
        gc.collect()
        self.assertIsNone(refs[0]())
        with self.assertRaises(ReferenceError):
            refs[1].cycle

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

    def test_internal_callback_is_not_called(self):
        callbacks = []

        class Class:
            pass

        obj = Class()
        obj.cycle = obj
        obj.ref = weakref.ref(obj, lambda wr: callbacks.append("called"))
        del obj
        gc.collect()
        self.assertEqual(callbacks, [])

    def test_callback_edge_is_traversed(self):
        class Class:
            pass

        obj = Class()
        callback = lambda wr: None
        for wr in (weakref.ref(obj, callback), weakref.proxy(obj, callback)):
            self.assertTrue(gc.is_tracked(wr))
            self.assertEqual(gc.get_referents(wr), [callback])

    def test_callback_cycle_is_collectible(self):
        class Class:
            pass

        def create_cycle():
            obj = Class()
            obj.ref = weakref.ref(obj, lambda wr: obj)
            return weakref.ref(obj)

        observer = create_cycle()
        gc.collect()
        self.assertIsNone(observer())

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
