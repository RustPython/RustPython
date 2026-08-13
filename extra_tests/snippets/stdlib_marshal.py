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


if __name__ == "__main__":
    unittest.main()
