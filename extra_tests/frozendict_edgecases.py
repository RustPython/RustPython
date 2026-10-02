"""Original PEP 814 regressions checked against CPython 3.15.0rc2."""

import gc
import operator
import unittest
import weakref


class FrozenDictEdgeCaseTests(unittest.TestCase):
    def test_hash_matches_frozenset_of_items(self):
        cases = (
            frozendict(),
            frozendict([(1, 2), (-3, 4), (2**130, -7)]),
            frozendict(first=1, second=2),
            frozendict(second=2, first=1),
            frozendict([(1, "one"), ("two", 2), (("three",), frozenset({3}))]),
        )
        for frozen in cases:
            with self.subTest(frozen=frozen):
                self.assertEqual(hash(frozen), hash(frozenset(frozen.items())))

    def test_gc_cycles_and_class_reassignment_cannot_expose_mutation(self):
        frozen = frozendict(protected=23)
        original_hash = hash(frozen)
        with self.assertRaises(TypeError):
            frozen.__class__ = dict
        self.assertIs(type(frozen), frozendict)
        self.assertEqual(frozen, {"protected": 23})
        self.assertEqual(hash(frozen), original_hash)

        class Marker:
            pass

        def make_cycle(include_views):
            marker = Marker()
            loop = [marker]
            owner = frozendict(loop=loop)
            loop.append(owner)
            if include_views:
                loop.extend((owner.keys(), owner.values(), owner.items(), iter(owner)))
            return weakref.ref(marker)

        for include_views in (False, True):
            with self.subTest(include_views=include_views):
                marker_ref = make_cycle(include_views)
                gc.collect()
                self.assertIsNone(marker_ref())

    def test_union_subclass_dispatch_matrix(self):
        for base in (dict, frozendict):

            class NativeIteration(base):
                def keys(self):
                    return ["virtual"]

                def __getitem__(self, key):
                    return 88

            class CustomIteration(NativeIteration):
                def __iter__(self):
                    return iter(["virtual"])

            for cls in (NativeIteration, CustomIteration):
                custom = cls is CustomIteration
                for storage in ({}, {"stored": 7}):
                    subject = cls(storage)
                    copied = {"virtual": 88} if custom and storage else storage
                    merged = {"virtual": 88} if custom else storage
                    cases = (
                        ("left_dict", lambda: subject | {}, base, copied),
                        (
                            "left_frozen",
                            lambda: subject | frozendict(),
                            base,
                            copied,
                        ),
                        ("right_dict", lambda: {} | subject, dict, merged),
                        (
                            "right_frozen",
                            lambda: frozendict() | subject,
                            frozendict,
                            merged,
                        ),
                        (
                            "construction",
                            lambda: frozendict(subject),
                            frozendict,
                            merged,
                        ),
                    )
                    for name, operation, expected_type, expected in cases:
                        with self.subTest(
                            base=base, cls=cls, storage=storage, operation=name
                        ):
                            result = operation()
                            self.assertIs(type(result), expected_type)
                            self.assertEqual(result, expected)
                            self.assertEqual(
                                list(subject.items()), list(storage.items())
                            )

                    if base is frozendict:
                        with self.subTest(
                            base=base, cls=cls, storage=storage, operation="copy"
                        ):
                            result = subject.copy()
                            self.assertIs(type(result), frozendict)
                            self.assertIsNot(result, subject)
                            self.assertEqual(result, copied)

    def test_mapping_equality_uses_left_value_and_stored_key_hash(self):
        trace = []

        class Key:
            def __init__(self, side):
                self.side = side

            def __hash__(self):
                trace.append(("hash", self.side))
                return 313

            def __eq__(self, other):
                trace.append(("key", self.side, other.side))
                return True

        class Value:
            def __init__(self, side):
                self.side = side

            def __eq__(self, other):
                trace.append(("value", self.side, other.side))
                return self.side == "left"

        for left_type in (dict, frozendict):
            for right_type in (dict, frozendict):
                left = left_type([(Key("left"), Value("left"))])
                right = right_type([(Key("right"), Value("right"))])
                with self.subTest(left_type=left_type, right_type=right_type):
                    trace.clear()
                    self.assertTrue(left == right)
                    self.assertEqual(
                        trace,
                        [("key", "right", "left"), ("value", "left", "right")],
                    )
                    trace.clear()
                    self.assertFalse(right == left)
                    self.assertEqual(
                        trace,
                        [("key", "left", "right"), ("value", "right", "left")],
                    )

    def test_view_comparison_direction_and_asymmetric_values(self):
        trace = []

        class Key:
            def __init__(self, side):
                self.side = side

            def __hash__(self):
                trace.append(("hash", self.side))
                return 313

            def __eq__(self, other):
                trace.append(("key", self.side, other.side))
                return True

        class Value:
            def __init__(self, side):
                self.side = side

            def __eq__(self, other):
                trace.append(("value", self.side, other.side))
                return self.side == "left"

        for left_type in (dict, frozendict):
            for right_type in (dict, frozendict):
                left = left_type([(Key("left"), Value("left"))])
                right = right_type([(Key("right"), Value("right"))])
                for method in ("keys", "items"):
                    for op in (
                        operator.eq,
                        operator.ne,
                        operator.lt,
                        operator.le,
                        operator.gt,
                        operator.ge,
                    ):
                        with self.subTest(
                            left_type=left_type,
                            right_type=right_type,
                            method=method,
                            operation=op.__name__,
                        ):
                            trace.clear()
                            result = op(
                                getattr(left, method)(), getattr(right, method)()
                            )
                            if op in (operator.lt, operator.gt):
                                self.assertFalse(result)
                                self.assertEqual(trace, [])
                                continue
                            reverse = op is operator.ge
                            needle, container = (
                                ("right", "left") if reverse else ("left", "right")
                            )
                            expected_trace = [
                                ("hash", needle),
                                ("key", container, needle),
                            ]
                            contained = True
                            if method == "items":
                                expected_trace.append(("value", container, needle))
                                contained = container == "left"
                            self.assertEqual(
                                result,
                                not contained if op is operator.ne else contained,
                            )
                            self.assertEqual(trace, expected_trace)

    def test_view_hash_error_context_comes_from_containment_mapping(self):
        class Key:
            broken = False

            def __hash__(self):
                if self.broken:
                    raise TypeError("intentional view hash error")
                return 313

        for left_type in (dict, frozendict):
            for right_type in (dict, frozendict):
                key = Key()
                left = left_type([(key, 1)])
                right = right_type([(key, 1)])
                key.broken = True
                for method in ("keys", "items"):
                    for op in (operator.eq, operator.ne, operator.le, operator.ge):
                        with self.subTest(
                            left_type=left_type,
                            right_type=right_type,
                            method=method,
                            operation=op.__name__,
                        ):
                            container = left_type if op is operator.ge else right_type
                            with self.assertRaises(TypeError) as caught:
                                op(getattr(left, method)(), getattr(right, method)())
                            self.assertTrue(
                                str(caught.exception).endswith(
                                    f"as a {container.__name__} key (intentional view hash error)"
                                ),
                                str(caught.exception),
                            )

    def test_view_equality_checks_mutation_of_the_iterated_mapping(self):
        for op in (operator.eq, operator.le, operator.ge):
            left = {"key": object()}

            class Value:
                def __eq__(self, other):
                    left.clear()
                    return True

            right = frozendict(key=Value())
            with self.subTest(operation=op.__name__):
                if op is operator.ge:
                    self.assertTrue(op(left.items(), right.items()))
                else:
                    with self.assertRaisesRegex(
                        RuntimeError, "dictionary changed size during iteration"
                    ):
                        op(left.items(), right.items())
                self.assertEqual(left, {})

    def test_fromkeys_merges_constructor_result_before_iterable_validation(self):
        calls = []

        class Source(frozendict):
            def __iter__(self):
                return iter(())

            def keys(self):
                calls.append("keys")
                return []

        source = Source()

        class Factory(frozendict):
            def __new__(cls, *args):
                calls.append("new")
                return source

        with self.assertRaises(TypeError):
            Factory.fromkeys(123)
        self.assertEqual(calls, ["new", "keys"])
        self.assertEqual(source, {})

    def test_fromkeys_source_error_precedes_invalid_iterable(self):
        class Source(frozendict):
            def __iter__(self):
                return iter(())

            def keys(self):
                raise ValueError("source merge failed")

        source = Source()

        class Factory(frozendict):
            def __new__(cls, *args):
                return source

        with self.assertRaisesRegex(ValueError, "source merge failed"):
            Factory.fromkeys(123)


if __name__ == "__main__":
    unittest.main()
