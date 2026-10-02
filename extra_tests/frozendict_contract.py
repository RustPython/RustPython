"""Original PEP 814 regressions, characterized against CPython 3.15.0rc2.

Run directly with RustPython or CPython 3.15, or through tests/frozendict.rs.
These tests intentionally do not import or alter upstream tests.
"""

import builtins
import copy
import gc
import pickle
import types
import unittest


class FrozenWithState(frozendict):
    pass


class FrozenWithSlot(frozendict):
    __slots__ = ("extra",)


class FrozenDictContractTests(unittest.TestCase):
    def test_construction_identity_and_explicit_new(self):
        original = frozendict(left=object())
        self.assertIs(frozendict(original), original)
        explicit = frozendict.__new__(frozendict, original)
        self.assertIsNot(explicit, original)
        self.assertEqual(explicit, original)
        self.assertIsNot(FrozenWithState(original), original)
        self.assertIsNot(frozendict(FrozenWithState(original)), original)
        original.__init__(object(), object(), ignored=object())
        self.assertEqual(list(original), ["left"])
        self.assertFalse(isinstance(original, dict))
        self.assertFalse(issubclass(frozendict, dict))

    def test_call_ex_identity_preserves_explicit_type_call_distinction(self):
        original = frozendict(answer=42)
        self.assertIs(frozendict(*(original,)), original)
        self.assertIs(frozendict(original, **{}), original)  # noqa: PIE804
        self.assertIs(frozendict(*(original,), **{}), original)  # noqa: PIE804
        self.assertIsNot(type.__call__(frozendict, original), original)
        self.assertIsNot(frozendict.__call__(original), original)

    def test_unrelated_descriptors_cannot_mutate_or_access(self):
        frozen = frozendict(protected=23)
        for method, args in (
            (dict.__init__, ({"replacement": 8},)),
            (dict.__setitem__, ("protected", 99)),
            (dict.__delitem__, ("protected",)),
            (dict.update, ({"replacement": 8},)),
            (dict.clear, ()),
            (dict.get, ("protected",)),
        ):
            with self.subTest(method=method), self.assertRaises(TypeError):
                method(frozen, *args)
        with self.assertRaises(TypeError):
            frozendict.get({"protected": 23}, "protected")
        self.assertEqual(frozen, {"protected": 23})
        for name in (
            "clear",
            "update",
            "pop",
            "popitem",
            "setdefault",
            "__setitem__",
            "__delitem__",
            "__ior__",
        ):
            self.assertFalse(hasattr(frozen, name), name)

    def test_subclass_missing_is_only_a_subscript_hook(self):
        class Lookup(frozendict):
            def __missing__(self, key):
                return "missing:" + key

        frozen = Lookup(present=23)
        self.assertEqual(frozen["absent"], "missing:absent")
        self.assertIsNone(frozen.get("absent"))
        self.assertNotIn("absent", frozen)

    def test_fromkeys_constructor_calls_and_source_independence(self):
        calls = []
        seed = frozendict(retained="old", replaced="old")

        class Factory(frozendict):
            def __new__(cls, *args):
                calls.append(args)
                if not args:
                    return seed
                return super().__new__(cls, *args)

            def __setitem__(self, key, value):
                raise AssertionError("immutable construction called a mutator")

        frozen = Factory.fromkeys(iter(["replaced", "added", "replaced"]), "new")
        self.assertIs(type(frozen), Factory)
        self.assertEqual(
            list(frozen.items()),
            [("retained", "old"), ("replaced", "new"), ("added", "new")],
        )
        self.assertEqual(seed, {"retained": "old", "replaced": "old"})
        self.assertEqual(len(calls), 2)
        self.assertEqual(calls[0], ())
        self.assertIs(type(calls[1][0]), frozendict)
        self.assertEqual(calls[1][0], frozen)

    def test_fromkeys_preserves_custom_constructor_result(self):
        target = {"seed": 1}

        class MutableFactory(frozendict):
            def __new__(cls):
                return target

        result = MutableFactory.fromkeys(["new"], 2)
        self.assertIs(result, target)
        self.assertEqual(target, {"seed": 1, "new": 2})

    def test_fromkeys_subclass_initialization_is_two_phase(self):
        calls = []

        class Initialized(frozendict):
            def __init__(self, *args):
                calls.append(args)

        result = Initialized.fromkeys(["first", "second"], 3)
        self.assertIs(type(result), Initialized)
        self.assertEqual(len(calls), 2)
        self.assertEqual(calls[0], ())
        self.assertIs(type(calls[1][0]), frozendict)
        self.assertEqual(calls[1][0], result)

    def test_fromkeys_constructs_before_rejecting_invalid_iterable(self):
        calls = []

        class Factory(frozendict):
            def __new__(cls, *args):
                calls.append(args)
                return super().__new__(cls, *args)

        with self.assertRaises(TypeError):
            Factory.fromkeys(123)
        self.assertEqual(calls, [()])

    def test_hash_uses_stored_key_hash_and_caches_values(self):
        class Counted:
            def __init__(self, value):
                self.value = value
                self.calls = 0

            def __hash__(self):
                self.calls += 1
                return self.value

        key, value = Counted(41), Counted(73)
        frozen = frozendict([(key, value)])
        key_calls = key.calls
        first = hash(frozen)
        self.assertEqual(key.calls, key_calls)
        self.assertEqual(value.calls, 1)
        self.assertEqual(hash(frozen), first)
        self.assertEqual(key.calls, key_calls)
        self.assertEqual(value.calls, 1)

    def test_hash_failure_does_not_poison_cache(self):
        class EventuallyHashable:
            def __init__(self):
                self.calls = 0

            def __hash__(self):
                self.calls += 1
                if self.calls == 1:
                    raise ValueError("not yet")
                return 97

        value = EventuallyHashable()
        frozen = frozendict(retry=value)
        with self.assertRaisesRegex(ValueError, "not yet"):
            hash(frozen)
        expected = hash(frozen)
        self.assertEqual(hash(frozen), expected)
        self.assertEqual(value.calls, 2)

    def test_mapping_equality_reuses_hashes_but_views_rehash(self):
        class Key:
            forbidden = False

            def __hash__(self):
                if self.forbidden:
                    raise ValueError("rehash forbidden")
                return 313

        key = Key()
        left = frozendict([(key, 7)])
        right = frozendict.__new__(frozendict, left)
        mutable = dict(left)
        key.forbidden = True
        self.assertTrue(left == left)  # noqa: PLR0124
        self.assertTrue(left.__eq__(left))
        self.assertTrue(left == right)
        self.assertTrue(left == mutable)
        self.assertTrue(mutable == left)
        for method in ("keys", "items"):
            with self.assertRaisesRegex(ValueError, "rehash forbidden"):
                _ = getattr(left, method)() == getattr(right, method)()

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

    def test_union_result_type_order_and_identity(self):
        frozen = frozendict(first=1, second=2)
        result = frozen | {"first": 7, "last": 3}
        self.assertIs(type(result), frozendict)
        self.assertEqual(
            list(result.items()), [("first", 7), ("second", 2), ("last", 3)]
        )
        self.assertIs(type({"start": 4} | frozen), dict)
        self.assertIs(frozen | {}, frozen)
        self.assertIs(frozendict() | frozen, frozen)
        child = FrozenWithState(frozen)
        self.assertIs(type(child | {}), frozendict)
        self.assertIsNot(child | {}, child)
        before = frozen
        frozen |= {"third": 3}
        self.assertIsNot(frozen, before)
        self.assertNotIn("third", before)
        with self.assertRaises(TypeError):
            frozen |= [("fourth", 4)]

    def test_explicit_reflected_union_uses_left_operand_type_and_order(self):
        frozen = frozendict(left=1, shared="frozen")
        mutable = {"right": 2, "shared": "mutable"}
        result = dict.__ror__(mutable, frozen)
        self.assertIs(type(result), frozendict)
        self.assertEqual(
            list(result.items()), [("left", 1), ("shared", "mutable"), ("right", 2)]
        )
        result = frozendict.__ror__(frozen, mutable)
        self.assertIs(type(result), dict)
        self.assertEqual(
            list(result.items()), [("right", 2), ("shared", "frozen"), ("left", 1)]
        )

    def test_frozen_globals_and_separate_writable_locals(self):
        observed = []
        frozen = frozendict(answer=42, observed=observed, __builtins__=builtins)
        local = {}
        exec("observed.append(answer); local_answer = answer + 1", frozen, local)
        self.assertEqual(observed, [42])
        self.assertEqual(local, {"local_answer": 43})
        self.assertEqual(eval("answer + 2", frozen), 44)
        self.assertIs(eval("globals()", frozen), frozen)
        self.assertNotIn("local_answer", frozen)

    def test_frozen_globals_require_existing_builtins(self):
        for execute, source in ((eval, "40 + 2"), (exec, "pass")):
            for frozen in (frozendict(answer=42), FrozenWithState(answer=42)):
                with self.subTest(execute=execute, frozen_type=type(frozen)):
                    with self.assertRaisesRegex(
                        TypeError, "cannot assign __builtins__ to frozendict globals"
                    ):
                        execute(source, frozen, {})
                    self.assertNotIn("__builtins__", frozen)

    def test_functions_from_exec_keep_frozen_globals(self):
        frozen = frozendict(answer=42, __builtins__=builtins)
        local = {}
        exec(
            "def read():\n return answer\ndef assign():\n global answer\n answer = 99\ndef delete():\n global answer\n del answer",
            frozen,
            local,
        )
        self.assertIs(local["read"].__globals__, frozen)
        self.assertEqual(local["read"](), 42)
        for name, suffix in (("assign", "assignment"), ("delete", "deletion")):
            with self.assertRaises(TypeError) as caught:
                local[name]()
            self.assertEqual(
                str(caught.exception),
                "frozendict object does not support item " + suffix,
            )
        with self.assertRaises(TypeError):
            types.FunctionType(local["read"].__code__, frozen)
        self.assertEqual(frozen["answer"], 42)

    def test_frozen_builtins_and_type_namespace(self):
        frozen_builtins = frozendict(len=len)
        frozen = frozendict(__builtins__=frozen_builtins)
        self.assertEqual(eval("len((10, 20))", frozen), 2)
        namespace = frozendict(value=17)
        cls = type("FrozenNamespace", (), namespace)
        self.assertEqual(cls.value, 17)
        cls.value = 19
        self.assertEqual(namespace["value"], 17)

    def test_frozen_subclass_globals_keep_lookup_semantics(self):
        class Namespace(frozendict):
            def __getitem__(self, key):
                if key == "answer":
                    return 81
                return super().__getitem__(key)

        frozen = Namespace(answer=42, __builtins__=builtins)
        self.assertEqual(eval("answer", frozen), 81)
        self.assertEqual(eval("(lambda: answer)()", frozen), 81)
        self.assertEqual(eval("answer", frozen, {}), 42)

    def test_class_and_mapping_patterns(self):
        frozen = frozendict(answer=42)
        match frozen:
            case frozendict(captured):
                self.assertIs(captured, frozen)
            case _:
                self.fail("frozendict class pattern did not match")
        match frozen:
            case {"answer": captured, **remaining}:
                self.assertEqual(captured, 42)
                self.assertEqual(remaining, {})
                self.assertIs(type(remaining), dict)
            case _:
                self.fail("frozendict mapping pattern did not match")

    def test_hash_error_wrapper_preserves_exception_subclasses(self):
        class SpecialTypeError(TypeError):
            pass

        class BadKey:
            error = TypeError

            def __hash__(self):
                raise self.error("intentional hash failure")

        key = BadKey()
        frozen = frozendict(present=1)
        operations = (
            lambda: frozen[key],
            lambda: frozen.get(key),
            lambda: key in frozen,
            lambda: frozendict([(key, 1)]),
            lambda: frozendict.fromkeys([key]),
        )
        for error in (TypeError, SpecialTypeError, ValueError):
            key.error = error
            for operation in operations:
                with self.subTest(error=error, operation=operation):
                    with self.assertRaises(error) as caught:
                        operation()
                    self.assertIs(type(caught.exception), error)
                    expected = "intentional hash failure"
                    if error is TypeError:
                        name = BadKey.__qualname__
                        if BadKey.__module__ not in ("builtins", "__main__"):
                            name = BadKey.__module__ + "." + name
                        expected = f"cannot use '{name}' as a frozendict key (intentional hash failure)"
                    self.assertEqual(str(caught.exception), expected)

    def test_copy_protocol_and_getnewargs_do_not_expose_storage(self):
        frozen = frozendict(present=object())
        self.assertIs(copy.copy(frozen), frozen)
        self.assertIs(frozen.copy(), frozen)
        args = frozen.__getnewargs__()
        self.assertIs(type(args), tuple)
        self.assertIs(type(args[0]), dict)
        args[0].clear()
        self.assertEqual(list(frozen), ["present"])
        child = FrozenWithState(frozen)
        child.extra = "state"
        shallow = copy.copy(child)
        self.assertIs(type(shallow), FrozenWithState)
        self.assertEqual(shallow.extra, "state")
        self.assertIs(type(child.copy()), frozendict)
        deep = copy.deepcopy(frozen)
        self.assertIsNot(deep, frozen)

    def test_subclass_copy_dispatch_tracks_iterator_override(self):
        class NativeIteration(frozendict):
            def keys(self):
                return ["virtual"]

            def __getitem__(self, key):
                return 88

        class CustomIteration(NativeIteration):
            def __iter__(self):
                return iter(["virtual"])

        native = NativeIteration(stored=7)
        custom = CustomIteration(stored=7)
        self.assertEqual(native.copy(), {"stored": 7})
        self.assertEqual(native | {"tail": 9}, {"stored": 7, "tail": 9})
        self.assertEqual(NativeIteration.fromkeys(["tail"], 9), {"tail": 9})
        self.assertEqual(custom.copy(), {"virtual": 88})
        self.assertEqual(custom | {"tail": 9}, {"virtual": 88, "tail": 9})
        self.assertEqual(
            CustomIteration.fromkeys(["tail"], 9), {"virtual": 88, "tail": 9}
        )

    def test_deepcopy_recursive_aliases_from_both_roots(self):
        loop = []
        frozen = frozendict(left=loop, right=loop)
        loop.extend([frozen, frozen])
        for root in (frozen, loop):
            result = copy.deepcopy(root)
            copied_frozen = result if isinstance(result, frozendict) else result[0]
            copied_loop = copied_frozen["left"]
            self.assertIsNot(copied_frozen, frozen)
            self.assertIsNot(copied_loop, loop)
            self.assertIs(copied_frozen["right"], copied_loop)
            self.assertIs(copied_loop[0], copied_frozen)
            self.assertIs(copied_loop[1], copied_frozen)

    def test_pickle_recursive_payload_and_subclass_state(self):
        for cls in (frozendict, FrozenWithState, FrozenWithSlot):
            loop = []
            frozen = cls(left=loop, right=loop)
            loop.extend([frozen, frozen])
            if cls is not frozendict:
                frozen.extra = loop
            for protocol in range(pickle.HIGHEST_PROTOCOL + 1):
                with self.subTest(cls=cls, protocol=protocol):
                    if protocol < 2:
                        with self.assertRaises(TypeError):
                            pickle.dumps(frozen, protocol)
                        continue
                    result = pickle.loads(pickle.dumps(frozen, protocol))
                    self.assertIs(type(result), cls)
                    self.assertIsNot(result, frozen)
                    self.assertIs(result["left"], result["right"])
                    self.assertIs(result["left"][0], result)
                    self.assertIs(result["left"][1], result)
                    if cls is not frozendict:
                        self.assertIs(result.extra, result["left"])

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
