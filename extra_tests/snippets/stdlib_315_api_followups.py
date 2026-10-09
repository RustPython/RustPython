"""Bytearray reinitialization must recheck exports acquired by input callbacks."""

import unittest


class ApiFollowups(unittest.TestCase):
    def test_bytearray_reinitialization_rechecks_exports(self):
        data = bytearray(b"original")
        held = []

        class ExportingInput:
            def __iter__(self):
                held.append(memoryview(data))
                return iter(b"new")

        try:
            with self.assertRaises(BufferError):
                data.__init__(ExportingInput())
        finally:
            for view in held:
                view.release()


if __name__ == "__main__":
    unittest.main()
