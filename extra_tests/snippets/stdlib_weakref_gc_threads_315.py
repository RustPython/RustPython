"""Threaded weakref resurrection and late-upgrade stress; run in a subprocess."""

import gc
import threading
import time
import unittest
import weakref


class WeakrefThreadTests(unittest.TestCase):
    def test_other_thread_can_resurrect_during_finalizer(self):
        finalizing = threading.Event()
        acquired = threading.Event()
        held = []
        errors = []
        marker = object()

        class Target:
            def __del__(self):
                finalizing.set()
                if not acquired.wait(5):
                    errors.append("thread did not finish weakref upgrade")

        obj = Target()
        obj.cycle = obj
        obj.marker = marker
        observer = weakref.ref(obj)

        def upgrade():
            if not finalizing.wait(5):
                errors.append("finalizer did not run")
            else:
                target = observer()
                if target is None:
                    errors.append("callback-free weakref cleared before finalizer")
                else:
                    held.append(target)
            acquired.set()

        thread = threading.Thread(target=upgrade)
        thread.start()
        del obj
        gc.collect()
        thread.join(5)
        self.assertFalse(thread.is_alive())
        self.assertEqual(errors, [])
        self.assertEqual(len(held), 1)
        self.assertIs(observer(), held[0])
        self.assertIs(held[0].marker, marker)
        held.clear()
        gc.collect()
        self.assertIsNone(observer())

    def test_concurrent_upgrade_never_sees_cleared_payload(self):
        enabled = gc.isenabled()
        gc.disable()
        marker = object()
        refs = ()
        stop = threading.Event()
        ready = threading.Event()
        errors = []

        class Target:
            pass

        def upgrade():
            ready.set()
            while not stop.is_set():
                for ref in refs:
                    obj = ref()
                    if obj is not None:
                        time.sleep(0)
                        if getattr(obj, "marker", None) is not marker:
                            errors.append("live upgraded reference has cleared payload")
                        del obj
                time.sleep(0)

        worker = threading.Thread(target=upgrade)
        worker.start()
        try:
            self.assertTrue(ready.wait(5))
            for _ in range(30):
                batch = []
                for _ in range(64):
                    obj = Target()
                    obj.marker = marker
                    obj.cycle = obj
                    batch.append(weakref.ref(obj))
                del obj
                refs = tuple(batch)
                time.sleep(0)
                gc.collect()
            self.assertEqual(errors, [])
        finally:
            stop.set()
            worker.join(5)
            if enabled:
                gc.enable()
        self.assertFalse(worker.is_alive())


if __name__ == "__main__":
    unittest.main(verbosity=2)
