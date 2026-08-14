import marshal
import unittest


class MarshalTests(unittest.TestCase):
    """
    Testing the (incomplete) marshal module.
    """

    def dump_then_load(self, data):
        return marshal.loads(marshal.dumps(data))

    def _test_marshal(self, data):
        self.assertEqual(self.dump_then_load(data), data)

    def test_marshal_int(self):
        self._test_marshal(0)
        self._test_marshal(-1)
        self._test_marshal(1)
        self._test_marshal(100000000)

    def test_marshal_bool(self):
        self._test_marshal(True)
        self._test_marshal(False)

    def test_marshal_float(self):
        self._test_marshal(0.0)
        self._test_marshal(-10.0)
        self._test_marshal(10.0)

    def test_marshal_str(self):
        self._test_marshal("")
        self._test_marshal("Hello, World")

    def test_marshal_list(self):
        self._test_marshal([])
        self._test_marshal([1, "hello", 1.0])
        self._test_marshal([[0], ["a", "b"]])

    def test_marshal_tuple(self):
        self._test_marshal(())
        self._test_marshal((1, "hello", 1.0))

    def test_marshal_dict(self):
        self._test_marshal({})
        self._test_marshal({"a": 1, 1: "a"})
        self._test_marshal({"a": {"b": 2}, "c": [0.0, 4.0, 6, 9]})

    def test_marshal_set(self):
        self._test_marshal(set())
        self._test_marshal({1, 2, 3})
        self._test_marshal({1, "a", "b"})

    def test_marshal_frozen_set(self):
        self._test_marshal(frozenset())
        self._test_marshal(frozenset({1, 2, 3}))
        self._test_marshal(frozenset({1, "a", "b"}))

    def test_marshal_bytearray(self):
        self.assertEqual(
            self.dump_then_load(bytearray([])),
            bytearray(b""),
        )
        self.assertEqual(
            self.dump_then_load(bytearray([1, 2])),
            bytearray(b"\x01\x02"),
        )

    def test_roundtrip(self):
        orig = compile("1 + 1", "", "eval")

        dumped = marshal.dumps(orig)
        loaded = marshal.loads(dumped)

        assert eval(loaded) == eval(orig)

    def test_roundtrip_non_constant_co_consts(self):
        # `code.replace` accepts any marshalable object, including values the
        # compiler constant representation cannot describe.
        orig = compile("1 + 1", "", "eval").replace(
            co_consts=([1, 2], {"a": 3}, {4, 5}, 6)
        )

        loaded = marshal.loads(marshal.dumps(orig))

        self.assertEqual(loaded.co_consts, ([1, 2], {"a": 3}, {4, 5}, 6))

    def test_roundtrip_shared_co_const(self):
        # A constant shared with the enclosing object is written once and both
        # readers resolve the same reference.
        shared = ["shared"]
        orig = compile("1 + 1", "", "eval").replace(co_consts=(shared,))

        loaded_code, loaded_shared = marshal.loads(marshal.dumps((orig, shared)))

        self.assertIs(loaded_code.co_consts[0], loaded_shared)


class AllowCodeTests(unittest.TestCase):
    """allow_code is answered where a code object is written or read, so a
    graph that walks back on itself is not a second walk of its own."""

    def test_recursive_value(self):
        recursive = []
        recursive.append(recursive)
        loaded = marshal.loads(
            marshal.dumps(recursive, allow_code=False), allow_code=False
        )
        self.assertIs(loaded[0], loaded)

    def test_too_deeply_nested(self):
        nested = []
        for _ in range(100_000):
            nested = [nested]
        with self.assertRaises(ValueError):
            marshal.dumps(nested, allow_code=False)

    def test_code_is_rejected(self):
        code = compile("1", "", "exec")
        for value in (code, [code], (code,), {0: code}):
            with self.assertRaises(ValueError):
                marshal.dumps(value, allow_code=False)
            data = marshal.dumps(value)
            with self.assertRaises(ValueError):
                marshal.loads(data, allow_code=False)


class BadDataTests(unittest.TestCase):
    def test_container_size_out_of_range(self):
        import struct

        # a length is signed, so the top bit set is out of range rather than
        # four billion items to reserve room for
        for marker in b"([<>":
            data = bytes([marker | 0x80]) + struct.pack("<I", 0xFFFFFF00)
            with self.assertRaises(ValueError):
                marshal.loads(data)


if __name__ == "__main__":
    unittest.main()
