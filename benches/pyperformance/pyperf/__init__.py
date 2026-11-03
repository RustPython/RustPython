"""A stand-in for ``pyperf`` used by the vendored pyperformance benchmarks.

The upstream ``run_benchmark.py`` files are kept byte-for-byte identical to
python/pyperformance; the only thing redirected is ``import pyperf``, which the
Criterion harness in ``benches/pyperformance.rs`` resolves to this module by
putting ``benches/pyperformance/`` on ``sys.path``.

The real ``pyperf.Runner`` spawns worker processes, calibrates loop counts and
collects timings. None of that happens here: every ``bench_*`` method runs the
benchmark body exactly once and returns. Repetition and measurement belong to
Criterion, which calls the whole script repeatedly.

Only the pyperf surface the vendored benchmarks use is implemented. Anything
else is a deliberate ``AttributeError`` so a new upstream API shows up as a
failure instead of as a benchmark that measures nothing.
"""

import argparse
import asyncio
import platform
import sys
import time

__all__ = ["Runner", "perf_counter", "python_implementation"]

perf_counter = time.perf_counter


def python_implementation():
    return platform.python_implementation().lower()


class Runner:
    """Accepts ``pyperf.Runner``'s keyword arguments and ignores them all
    except ``add_cmdline_args``, which upstream benchmarks use to forward their
    own options to worker processes; we call it so that code path runs."""

    def __init__(self, add_cmdline_args=None, **_ignored):
        self.argparser = argparse.ArgumentParser()
        self.metadata = {}
        self.args = None
        self._add_cmdline_args = add_cmdline_args

    def parse_args(self, args=None):
        if args is None:
            args = sys.argv[1:]
        self.args = self.argparser.parse_args(args)
        if self._add_cmdline_args is not None:
            self._add_cmdline_args([], self.args)
        return self.args

    def bench_func(self, name, func, *args, **_kwargs):
        func(*args)

    def bench_time_func(self, name, func, *args, **_kwargs):
        func(1, *args)

    def bench_async_func(self, name, func, *args, **_kwargs):
        asyncio.run(func(*args))

    def bench_command(self, name, command, **_kwargs):
        raise NotImplementedError(
            "pyperf.Runner.bench_command measures a subprocess; "
            "process-startup benchmarks are not run in-process"
        )
