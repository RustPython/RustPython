"""Native frozendict hash reentry and GC ownership regressions."""

import gc
import unittest


class FrozenDictContractTests(unittest.TestCase):
    def test_value_hash_can_reenter_same_frozendict(self):
        class Reentrant:
            def __init__(self):
                self.calls = 0
                self.owner = None
                self.nested = None

            def __hash__(self):
                self.calls += 1
                if self.calls == 1:
                    self.nested = hash(self.owner)
                return 127

        value = Reentrant()
        frozen = frozendict(recursive=value)
        value.owner = frozen
        result = hash(frozen)
        self.assertEqual(result, value.nested)
        self.assertEqual(hash(frozen), result)
        self.assertEqual(value.calls, 2)

    def test_gc_does_not_expose_mutable_backing_mapping(self):
        marker = object()
        frozen = frozendict(private_payload=marker)
        for referent in gc.get_referents(frozen):
            self.assertFalse(isinstance(referent, dict))
        self.assertIs(frozen["private_payload"], marker)

    def test_gc_does_not_publish_partly_constructed_mapping(self):
        marker = object()
        observed = []

        def source():
            yield "private_payload", marker
            observed.extend(
                candidate
                for candidate in gc.get_objects()
                if type(candidate) is frozendict
                and candidate.get("private_payload") is marker
            )
            yield "finished", True

        frozen = frozendict(source())
        self.assertEqual(observed, [])
        self.assertIs(frozen["private_payload"], marker)
        self.assertIs(frozen["finished"], True)


if __name__ == "__main__":
    unittest.main()
