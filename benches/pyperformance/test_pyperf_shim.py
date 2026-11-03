"""Tests for the ``pyperf`` shim that the vendored pyperformance benchmarks import.

Run under CPython or RustPython from the repository root:

    python3 -m unittest benches/pyperformance/test_pyperf_shim.py
    target/release/rustpython -m unittest benches/pyperformance/test_pyperf_shim.py

The shim runs each benchmark body exactly once and leaves timing and repetition
to the Criterion harness in ``benches/pyperformance.rs``.
"""

import asyncio
import os
import sys
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

import pyperf  # noqa: E402


class RunnerBenchTests(unittest.TestCase):
    def test_bench_func_calls_once_with_args(self):
        calls = []
        runner = pyperf.Runner()
        result = runner.bench_func("name", lambda *a: calls.append(a) or 42, 1, "two")
        self.assertEqual(calls, [(1, "two")])
        self.assertIsNone(result)

    def test_bench_func_ignores_inner_loops(self):
        calls = []
        pyperf.Runner().bench_func("name", lambda: calls.append(()), inner_loops=20)
        self.assertEqual(calls, [()])

    def test_bench_time_func_passes_one_loop(self):
        calls = []

        def timed(loops, *args):
            calls.append((loops, args))
            return 0.0

        pyperf.Runner().bench_time_func("name", timed, "x", 3)
        self.assertEqual(calls, [(1, ("x", 3))])

    def test_bench_async_func_runs_coroutine_once(self):
        calls = []

        async def coro(*args):
            await asyncio.sleep(0)
            calls.append(args)

        pyperf.Runner().bench_async_func("name", coro, 7)
        self.assertEqual(calls, [(7,)])

    def test_bench_command_is_not_supported(self):
        with self.assertRaises(NotImplementedError):
            pyperf.Runner().bench_command("name", [sys.executable, "-c", "pass"])


class RunnerArgsTests(unittest.TestCase):
    def setUp(self):
        self.saved_argv = sys.argv

    def tearDown(self):
        sys.argv = self.saved_argv

    def test_parse_args_reads_sys_argv(self):
        sys.argv = ["run_benchmark.py", "--digits", "500"]
        runner = pyperf.Runner()
        runner.argparser.add_argument("--digits", type=int, default=2000)
        args = runner.parse_args()
        self.assertEqual(args.digits, 500)
        self.assertIs(runner.args, args)

    def test_parse_args_accepts_explicit_list(self):
        runner = pyperf.Runner()
        runner.argparser.add_argument("benchmark", choices=["pickle", "unpickle"])
        self.assertEqual(runner.parse_args(["unpickle"]).benchmark, "unpickle")

    def test_parse_args_calls_add_cmdline_args(self):
        seen = []

        def add_cmdline_args(cmd, args):
            seen.append((list(cmd), args.flag))

        sys.argv = ["run_benchmark.py", "--flag"]
        runner = pyperf.Runner(add_cmdline_args=add_cmdline_args)
        runner.argparser.add_argument("--flag", action="store_true")
        runner.parse_args()
        self.assertEqual(len(seen), 1)
        self.assertTrue(seen[0][1])

    def test_runner_accepts_upstream_keywords(self):
        runner = pyperf.Runner(values=10, processes=2, warmups=1)
        self.assertEqual(runner.metadata, {})
        runner.metadata["description"] = "x"
        self.assertEqual(runner.metadata["description"], "x")


class ModuleTests(unittest.TestCase):
    def test_perf_counter_is_time_perf_counter(self):
        self.assertIs(pyperf.perf_counter, time.perf_counter)

    def test_python_implementation_is_lowercase_platform_name(self):
        import platform

        self.assertEqual(
            pyperf.python_implementation(), platform.python_implementation().lower()
        )

    def test_unknown_attribute_raises(self):
        with self.assertRaises(AttributeError):
            pyperf.Benchmark  # noqa: B018


if __name__ == "__main__":
    unittest.main()
