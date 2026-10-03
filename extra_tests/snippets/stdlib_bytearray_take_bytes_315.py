"""CPython 3.15 take_bytes semantics, including reentrant index conversion."""

import sys
import unittest


class TakeBytesTests(unittest.TestCase):
    def test_all_bytes_transfer(self):
        data = bytearray(range(256))
        result = data.take_bytes()
        self.assertIs(type(result), bytes)
        self.assertEqual(result, bytes(range(256)))
        self.assertEqual(data, b"")
        self.assertEqual(data.__alloc__(), 0)

    def test_prefix_and_negative_counts(self):
        data = bytearray(b"abcdef")
        self.assertEqual(data.take_bytes(1), b"a")
        self.assertEqual(data.take_bytes(-5), b"")
        self.assertEqual(data, b"bcdef")
        self.assertEqual(data.take_bytes(-3), b"bc")
        self.assertEqual(data, b"def")
        self.assertEqual(data.take_bytes(3), b"def")
        self.assertEqual(data, b"")

    def test_empty_and_none(self):
        for args in ((), (0,), (None,)):
            data = bytearray()
            self.assertEqual(data.take_bytes(*args), b"")
        data = bytearray(b"ab")
        self.assertEqual(data.take_bytes(None), b"ab")

    def test_index_and_bool(self):
        class Index:
            def __index__(self):
                return 2

        data = bytearray(b"abcd")
        self.assertEqual(data.take_bytes(Index()), b"ab")
        self.assertEqual(data.take_bytes(False), b"")
        self.assertEqual(data.take_bytes(True), b"c")

        class IntSubclass(int):
            def __index__(self):
                raise AssertionError("integer subclass override must be ignored")

        self.assertEqual(data.take_bytes(IntSubclass(1)), b"d")

    def test_bounds_and_overflow(self):
        for n in (-7, 7, -sys.maxsize - 2, sys.maxsize + 1):
            with self.subTest(n=n):
                data = bytearray(b"abcdef")
                with self.assertRaises(IndexError):
                    data.take_bytes(n)
                self.assertEqual(data, b"abcdef")
        with self.assertRaises(IndexError):
            bytearray().take_bytes(-1)

    def test_type_and_index_errors(self):
        for n in (3.14, "1", [], object()):
            with self.assertRaisesRegex(TypeError, "n must be an integer or None"):
                bytearray(b"ab").take_bytes(n)

        class Broken:
            def __index__(self):
                raise RuntimeError("index failed")

        with self.assertRaisesRegex(RuntimeError, "index failed"):
            bytearray(b"ab").take_bytes(Broken())

        class Wrong:
            def __index__(self):
                return 1.5

        with self.assertRaises(TypeError):
            bytearray(b"ab").take_bytes(Wrong())

    def test_positional_only(self):
        with self.assertRaises(TypeError):
            bytearray(b"ab").take_bytes(n=1)
        with self.assertRaises(TypeError):
            bytearray(b"ab").take_bytes(1, 2)

    def test_every_take_rejects_exports(self):
        for payload in (b"", b"ab"):
            for args in ((), (0,), (None,)):
                with self.subTest(payload=payload, args=args):
                    data = bytearray(payload)
                    with memoryview(data):
                        with self.assertRaises(BufferError):
                            data.take_bytes(*args)
                        self.assertEqual(data, payload)
                    self.assertEqual(data.take_bytes(), payload)

    def test_argument_errors_precede_buffer_errors(self):
        data = bytearray(b"ab")
        with memoryview(data):
            for n in (-3, 3, sys.maxsize + 1):
                with self.assertRaises(IndexError):
                    data.take_bytes(n)
            with self.assertRaises(TypeError):
                data.take_bytes("1")
        self.assertEqual(data, b"ab")

    def test_reentrant_resize_uses_current_size(self):
        def take(data, action, n):
            class Index:
                def __index__(self):
                    action(data)
                    return n

            return data.take_bytes(Index())

        data = bytearray(b"abcdefgh")
        with self.assertRaises(IndexError):
            take(data, bytearray.clear, 8)
        self.assertEqual(data, b"")
        data = bytearray(b"abcdefgh")
        with self.assertRaises(IndexError):
            take(data, lambda b: b.__delitem__(slice(4, None)), 8)
        self.assertEqual(data, b"abcd")
        data = bytearray(b"abcd")
        self.assertEqual(take(data, lambda b: b.extend(b"efgh"), 8), b"abcdefgh")
        data = bytearray(b"abcd")
        self.assertEqual(take(data, lambda b: b.extend(b"efgh"), -4), b"abcd")
        self.assertEqual(data, b"efgh")

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

    def test_subclass_uses_underlying_storage(self):
        class Subclass(bytearray):
            def __len__(self):
                raise AssertionError("no __len__ override")

            def __getitem__(self, index):
                raise AssertionError("no __getitem__ override")

            def __bytes__(self):
                raise AssertionError("no __bytes__ override")

        data = Subclass(b"abcd")
        data.metadata = "kept"
        result = data.take_bytes(2)
        self.assertIs(type(result), bytes)
        self.assertEqual(result, b"ab")
        self.assertEqual(data, b"cd")
        self.assertIs(type(data), Subclass)
        self.assertEqual(data.metadata, "kept")

    def test_result_remains_immutable(self):
        data = bytearray(b"abcd")
        first = data.take_bytes(2)
        data[0] = 120
        rest = data.take_bytes()
        data.extend(b"new data")
        self.assertEqual(first, b"ab")
        self.assertEqual(rest, b"xd")

    def test_repeated_partial_takes(self):
        data = bytearray(range(100))
        chunks = [data.take_bytes(2) for _ in range(50)]
        self.assertEqual(b"".join(chunks), bytes(range(100)))
        self.assertEqual(data, b"")


if __name__ == "__main__":
    unittest.main(verbosity=2)
