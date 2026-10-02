import builtins
import gc
import sys
import types
import unittest


class FrozenGlobalsTests(unittest.TestCase):
    def setUp(self):
        self.ns = frozendict(
            __builtins__=builtins.__dict__, __name__="frozen_test", answer=42, sys=sys
        )

    def test_eval_and_implicit_scope(self):
        self.assertIs(eval("globals()", self.ns), self.ns)
        self.assertIs(eval("locals()", self.ns), self.ns)
        self.assertEqual(eval("eval('answer')", self.ns), 42)
        self.assertEqual(eval("(lambda: answer)()", self.ns), 42)

    def test_function_and_materialized_frame(self):
        local = {}
        exec("def f():\n return sys._getframe(), globals(), answer\n", self.ns, local)
        f = local["f"]
        self.assertIs(f.__globals__, self.ns)
        self.assertEqual(f.__module__, "frozen_test")
        for _ in range(300):
            frame, ns, answer = f()
            self.assertIs(frame.f_globals, self.ns)
            self.assertIs(ns, self.ns)
            self.assertEqual(answer, 42)
        gc.collect()
        self.assertIs(frame.f_globals, self.ns)

    def test_generator_frames(self):
        local = {}
        exec(
            "def gen():\n yield sys._getframe(), globals(), answer\n yield answer\n",
            self.ns,
            local,
        )
        gen = local["gen"]()
        self.assertIs(gen.gi_frame.f_globals, self.ns)
        frame, ns, answer = next(gen)
        self.assertIs(frame.f_globals, self.ns)
        self.assertIs(ns, self.ns)
        self.assertEqual(answer, 42)
        self.assertEqual(next(gen), 42)
        with self.assertRaises(StopIteration):
            next(gen)
        gc.collect()
        self.assertIs(frame.f_globals, self.ns)

    def test_coroutine_frames(self):
        local = {}
        exec(
            "async def coro():\n return sys._getframe(), globals(), answer\n",
            self.ns,
            local,
        )
        coro = local["coro"]()
        self.assertIs(coro.cr_frame.f_globals, self.ns)
        with self.assertRaises(StopIteration) as stopped:
            coro.send(None)
        frame, ns, answer = stopped.exception.value
        self.assertIs(frame.f_globals, self.ns)
        self.assertIs(ns, self.ns)
        self.assertEqual(answer, 42)

    def test_async_generator_frames(self):
        local = {}
        exec(
            "async def agen():\n yield sys._getframe(), globals(), answer\n",
            self.ns,
            local,
        )
        agen = local["agen"]()
        self.assertIs(agen.ag_frame.f_globals, self.ns)
        with self.assertRaises(StopIteration) as stopped:
            agen.asend(None).send(None)
        frame, ns, answer = stopped.exception.value
        self.assertIs(frame.f_globals, self.ns)
        self.assertIs(ns, self.ns)
        self.assertEqual(answer, 42)
        with self.assertRaises(StopIteration):
            agen.aclose().send(None)

    def test_local_namespace_and_imports(self):
        local = {}
        exec(
            "import math\nclass C:\n value = answer\ndef f():\n import math\n return math.sqrt(answer)\n",
            self.ns,
            local,
        )
        self.assertEqual(local["C"].value, 42)
        self.assertEqual(local["C"].__module__, "frozen_test")
        self.assertEqual(local["f"](), local["math"].sqrt(42))
        self.assertNotIn("math", self.ns)

    def test_missing_builtins(self):
        for func, source in [(eval, ""), (exec, "")]:
            with self.assertRaises(TypeError) as caught:
                func(source, frozendict())
            self.assertEqual(
                str(caught.exception),
                "cannot assign __builtins__ to frozendict globals",
            )

    def test_writes_and_deletes(self):
        for source, error in [
            ("answer = 0", "'frozendict' object does not support item assignment"),
            (
                "global answer; answer = 0",
                "frozendict object does not support item assignment",
            ),
            (
                "global answer; del answer",
                "frozendict object does not support item deletion",
            ),
            (
                "global missing; del missing",
                "frozendict object does not support item deletion",
            ),
        ]:
            with self.assertRaises(TypeError) as caught:
                exec(source, self.ns)
            self.assertEqual(str(caught.exception), error)
        self.assertEqual(self.ns["answer"], 42)
        with self.assertRaises(NameError):
            exec("del answer", self.ns)

    def test_explicit_function_constructor_stays_dict_only(self):
        with self.assertRaises(TypeError):
            types.FunctionType((lambda: 42).__code__, self.ns)

    def test_subclass_name_and_global_lookup(self):
        class F(frozendict):
            def __getitem__(self, key):
                if key == "answer":
                    return 81
                return super().__getitem__(key)

        ns = F(self.ns)
        self.assertEqual(eval("answer", ns), 81)
        self.assertEqual(eval("(lambda: answer)()", ns), 81)
        self.assertEqual(eval("answer", ns, {}), 42)

    def test_subclass_function_metadata_reads_storage(self):
        class F(frozendict):
            def __getitem__(self, key):
                if key == "__name__":
                    return "wrong_name"
                if key == "__builtins__":
                    return {"len": lambda x: 99}
                return super().__getitem__(key)

        ns = F(self.ns)
        f = eval("lambda: len([])", ns)
        self.assertEqual(f.__module__, "frozen_test")
        self.assertIs(f.__globals__, ns)
        self.assertEqual(f(), 0)

    def test_frozen_builtins(self):
        ns = frozendict(__builtins__=frozendict({"len": len}))
        self.assertEqual(eval("(lambda: len([1, 2]))()", ns), 2)

    def test_shared_code_does_not_keep_mutable_global_specialization(self):
        code = compile("lambda: answer", "<globals-probe>", "eval")
        mutable = {"__builtins__": builtins.__dict__, "answer": 19}
        f = eval(code, mutable)
        for _ in range(300):
            self.assertEqual(f(), 19)
        frozen_f = eval(code, self.ns)
        for _ in range(300):
            self.assertEqual(frozen_f(), 42)
            self.assertEqual(f(), 19)
        mutable["answer"] = 23
        self.assertEqual(f(), 23)
        self.assertEqual(frozen_f(), 42)


if __name__ == "__main__":
    unittest.main(verbosity=2)
