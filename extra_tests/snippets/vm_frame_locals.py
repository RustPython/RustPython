"""Tests for frame-local mappings and PEP 667 snapshot semantics."""

import sys
import threading
import unittest


class FrameLocalsProxyTest(unittest.TestCase):
    def test_extra_locals_do_not_leak_into_unoptimized_namespace(self):
        namespace = {"sys": sys}
        exec(
            """
x = 1
proxy = [sys._getframe().f_locals for a in [0]][0]
proxy["x"] = 2
frame_value = sys._getframe().f_locals["x"]
namespace_value = x
proxy_value = proxy["x"]
""",
            namespace,
        )

        self.assertEqual(namespace["frame_value"], 1)
        self.assertEqual(namespace["namespace_value"], 1)
        self.assertEqual(namespace["proxy_value"], 2)

    def test_hidden_locals_take_precedence_over_extra_locals(self):
        namespace = {"sys": sys}
        exec(
            """
value = [
    (
        sys._getframe().f_locals.__setitem__("a", 99),
        locals()["a"],
    )[1]
    for a in [7]
][0]
""",
            namespace,
        )

        self.assertEqual(namespace["value"], 7)

    def test_proxy_views_preserve_colliding_extra_local(self):
        def inspect_proxy(proxy):
            proxy["a"] = 99
            return (
                len(proxy),
                list(proxy.keys()),
                list(proxy.values()),
                list(proxy.items()),
                dict(proxy),
                repr(proxy),
                proxy["a"],
            )

        namespace = {"inspect_proxy": inspect_proxy, "sys": sys}
        exec(
            "observed = [inspect_proxy(sys._getframe().f_locals) for a in [7]][0]",
            namespace,
        )

        self.assertEqual(
            namespace["observed"],
            (
                2,
                ["a", "a"],
                [7, 99],
                [("a", 7), ("a", 99)],
                {"a": 7},
                "{'a': 7}",
                7,
            ),
        )

    def test_optimized_locals_returns_fresh_snapshots(self):
        def snapshots():
            before = locals()
            x = 1
            after = locals()
            return before, after, before is after

        before, after, same = snapshots()
        self.assertEqual(before, {})
        self.assertEqual(after, {"before": before, "x": 1})
        self.assertFalse(same)

    def test_proxy_equality_uses_frame_identity(self):
        def make_proxy():
            return sys._getframe().f_locals

        first = make_proxy()
        second = make_proxy()
        self.assertNotEqual(first, second)

        def same_frame():
            frame = sys._getframe()
            return frame.f_locals == frame.f_locals

        self.assertTrue(same_frame())

    def test_proxy_keys_use_dict_hash_and_equality_rules(self):
        class EqualWithDifferentHash:
            def __hash__(self):
                return hash("x") ^ 1

            def __eq__(self, other):
                return other == "x"

        def exercise():
            x = 1
            proxy = sys._getframe().f_locals
            key = EqualWithDifferentHash()
            proxy[key] = 2
            return x, proxy[key], proxy["x"]

        self.assertEqual(exercise(), (1, 2, 1))

    def test_proxy_hash_comparison_and_union_dispatch(self):
        class Reflected:
            def __ror__(self, other):
                return ("reflected", type(other).__name__)

            def __eq__(self, other):
                return ("equal", type(other).__name__)

        def exercise():
            x = 1
            proxy = sys._getframe().f_locals
            reflected = Reflected()
            inplace = proxy
            inplace |= reflected
            proxy |= {"x": 3, "extra": 4}
            return (
                type(proxy).__hash__,
                proxy | {"right": 2},
                {"left": 2} | proxy,
                proxy | reflected,
                inplace,
                proxy == reflected,
                x,
                proxy["extra"],
            )

        (
            hash_method,
            merged,
            reflected_merged,
            reflected_or,
            reflected_inplace,
            reflected_equal,
            x,
            extra,
        ) = exercise()
        self.assertIsNone(hash_method)
        self.assertEqual(merged["x"], 3)
        self.assertEqual(merged["right"], 2)
        self.assertEqual(reflected_merged["left"], 2)
        self.assertEqual(reflected_merged["x"], 3)
        self.assertEqual(reflected_or, ("reflected", "FrameLocalsProxy"))
        self.assertEqual(reflected_inplace, ("reflected", "FrameLocalsProxy"))
        self.assertEqual(reflected_equal, ("equal", "FrameLocalsProxy"))
        self.assertEqual((x, extra), (3, 4))

        def get_proxy():
            return sys._getframe().f_locals

        proxy = get_proxy()
        with self.assertRaises(TypeError):
            hash(proxy)
        with self.assertRaisesRegex(TypeError, "FrameLocalsProxy.*list"):
            proxy | []

    def test_dict_subclass_protocol_matches_cpython(self):
        class DictSubclass(dict):
            def keys(self):
                return ["virtual"]

            def __getitem__(self, key):
                if key == "virtual":
                    return 42
                return super().__getitem__(key)

        def exercise():
            proxy = sys._getframe().f_locals
            proxy.update(DictSubclass(real=1))
            merged = proxy | DictSubclass(real=2)
            reflected = DictSubclass(real=3) | proxy
            proxy |= DictSubclass(real=4)
            return (
                proxy["virtual"],
                "real" in proxy,
                merged["real"],
                reflected["real"],
            )

        self.assertEqual(exercise(), (42, False, 2, 3))

    def test_clear_releases_values_without_finalizer_reentrancy_deadlock(self):
        events = []

        class ReentrantFinalizer:
            def __init__(self, frame, key):
                self.frame = frame
                self.key = key

            def __del__(self):
                events.append(self.frame.f_locals.get(self.key))

        def frame_with_extra():
            frame = sys._getframe()
            frame.f_locals["extra"] = ReentrantFinalizer(frame, "extra")
            return frame

        frame = frame_with_extra()
        frame.clear()
        self.assertEqual(events, [None])

        events.clear()

        def frame_with_overwritten_fast_local():
            frame = sys._getframe()
            # Occupies a fast-local slot so the write below overwrites it.
            value = ReentrantFinalizer(frame, "value")  # noqa: F841
            frame.f_locals["value"] = 42
            return frame

        frame = frame_with_overwritten_fast_local()
        self.assertEqual(events, [])
        frame.clear()
        self.assertEqual(len(events), 1)

    def test_unoptimized_frame_locals_are_readable_from_another_thread(self):
        ready = threading.Event()
        release = threading.Event()
        namespace = {"ready": ready, "release": release, "sys": sys}

        thread = threading.Thread(
            target=exec,
            args=(
                "frame = sys._getframe()\nready.set()\nrelease.wait()",
                namespace,
            ),
        )
        thread.start()
        self.assertTrue(ready.wait(5))
        try:
            self.assertIs(namespace["frame"].f_locals, namespace)
        finally:
            release.set()
            thread.join(5)
        self.assertFalse(thread.is_alive())

    def test_clear_executing_frame_from_another_thread(self):
        ready = threading.Event()
        release = threading.Event()
        frames = []

        def worker():
            frames.append(sys._getframe())
            ready.set()
            release.wait()

        thread = threading.Thread(target=worker)
        thread.start()
        self.assertTrue(ready.wait(5))
        try:
            with self.assertRaisesRegex(RuntimeError, "executing frame"):
                frames[0].clear()
        finally:
            release.set()
            thread.join(5)
        self.assertFalse(thread.is_alive())


class CodeReplaceLocalsPlusTest(unittest.TestCase):
    def test_rejects_varnames_that_disagree_with_default_nlocals(self):
        def function(value):
            return value

        code = function.__code__
        with self.assertRaises(ValueError):
            code.replace(co_varnames=())
        with self.assertRaises(ValueError):
            code.replace(co_varnames=("value", "extra"))

    def test_rebuilds_metadata_for_replaced_cellvars_and_freevars(self):
        def outer(value):
            return lambda: value

        code = outer.__code__
        replaced_cells = code.replace(co_cellvars=("replacement",))
        self.assertEqual(replaced_cells.co_cellvars, ("replacement",))

        plain_code = (lambda value: value).__code__
        replaced_frees = plain_code.replace(co_freevars=("replacement",))
        self.assertEqual(replaced_frees.co_freevars, ("replacement",))


if __name__ == "__main__":
    unittest.main()
