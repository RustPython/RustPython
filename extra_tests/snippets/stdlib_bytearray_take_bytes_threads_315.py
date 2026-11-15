"""Bounded export/transfer stress; successful exports must stay coherent."""

import threading
import time
import unittest


class TakeBytesThreadTests(unittest.TestCase):
    def test_new_exports_cannot_race_transfer(self):
        data = bytearray(b"x" * 1024)
        stop = threading.Event()
        ready = threading.Event()
        errors = []

        def read_exports():
            ready.set()
            try:
                while not stop.is_set():
                    with memoryview(data) as view:
                        size = len(view)
                        time.sleep(0)
                        value = view.tobytes()
                        if len(value) != size or value != b"x" * size:
                            errors.append(
                                "export length/content changed during transfer"
                            )
                    time.sleep(0)
            except BaseException as exc:
                errors.append((type(exc).__name__, str(exc)))

        worker = threading.Thread(target=read_exports)
        worker.start()
        try:
            self.assertTrue(ready.wait(5))
            for _ in range(300):
                try:
                    result = data.take_bytes()
                except BufferError:
                    pass
                else:
                    self.assertIn(result, (b"", b"x" * 1024))
                if not data:
                    try:
                        data.extend(b"x" * 1024)
                    except BufferError:
                        pass
                time.sleep(0)
        finally:
            stop.set()
            worker.join(5)
        self.assertFalse(worker.is_alive())
        self.assertEqual(errors, [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
