"""Focused native API gaps exposed by the pinned rc2 library suites."""

import abc
import unittest
import warnings


class ApiFollowups(unittest.TestCase):
    def test_abc_instance_without_class(self):
        class Base(metaclass=abc.ABCMeta):
            pass

        class Child(Base):
            def __getattribute__(self, name):
                if name == "__class__":
                    raise AttributeError(name)
                return super().__getattribute__(name)

        self.assertIsInstance(Child(), Base)

        class Broken(Base):
            def __getattribute__(self, name):
                if name == "__class__":
                    raise RuntimeError("class access")
                return super().__getattribute__(name)

        with self.assertRaisesRegex(RuntimeError, "class access"):
            isinstance(Broken(), Base)

    def test_replace_count_keyword(self):
        for kind in (bytes, bytearray):
            self.assertEqual(kind(b"aaa").replace(b"a", b"b", count=2), b"bba")
            with self.assertRaises(TypeError):
                kind(b"aaa").replace(b"a", b"b", 1, count=2)

    def test_bytearray_reinitialization_rejects_exports(self):
        for original in (b"", b"x"):
            for args in ((), (b"",), (b"x",), ("x", "ascii"), (0,), (1,)):
                data = bytearray(original)
                with memoryview(data):
                    with self.assertRaises(BufferError):
                        data.__init__(*args)
                    self.assertEqual(data, original)
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

    def test_warning_stack_beyond_frames(self):
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            warnings.warn("message", stacklevel=100000)
        self.assertEqual(
            (caught[0].module, caught[0].filename, caught[0].lineno),
            ("sys", "<sys>", 0),
        )

    def test_invalid_warning_class_diagnostic(self):
        with self.assertRaisesRegex(TypeError, "not class 'int'"):
            warnings.warn("message", int)


if __name__ == "__main__":
    unittest.main()
