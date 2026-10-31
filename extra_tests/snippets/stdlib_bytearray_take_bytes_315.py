"""take_bytes must validate buffer exports after reentrant index conversion."""

import unittest


class TakeBytesTests(unittest.TestCase):
    def test_reentrant_export_checked_after_index(self):
        data = bytearray(b"ab")
        views = []

        class Index:
            def __index__(self):
                views.append(memoryview(data))
                return 1

        try:
            with self.assertRaises(BufferError):
                data.take_bytes(Index())
            self.assertEqual(data, b"ab")
        finally:
            for view in views:
                view.release()
        view = memoryview(data)

        class ReleaseIndex:
            def __index__(self):
                view.release()
                return 1

        self.assertEqual(data.take_bytes(ReleaseIndex()), b"a")


if __name__ == "__main__":
    unittest.main()
