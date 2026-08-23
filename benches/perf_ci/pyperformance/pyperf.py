# Minimal stand-in for the `pyperf` module, used by the vendored pyperformance
# benchmark kernels in this directory when they run under RustPython for the
# PR performance gate (scripts/perf_ci.py).
#
# This is NOT the real pyperf (https://github.com/psf/pyperf). The real
# harness spawns worker subprocesses and does wall-clock statistics; the perf
# gate instead counts instructions with callgrind, so all we need is to
# execute each kernel a fixed, deterministic number of times inside a single
# process. Keeping the vendored kernels importing `pyperf` unmodified makes
# refreshing them from upstream pyperformance trivial.
#
# Supported surface (everything the vendored kernels use):
#   pyperf.perf_counter
#   pyperf.Runner(add_cmdline_args=...)
#   runner.metadata (a dict)
#   runner.argparser (a real argparse.ArgumentParser)
#   runner.parse_args()
#   runner.bench_func(name, func, *args)
#   runner.bench_time_func(name, time_func, *args)
#
# The number of kernel invocations is controlled by PERF_CI_LOOPS (default 1).

import argparse
import os
import sys
from time import perf_counter

LOOPS = int(os.environ.get("PERF_CI_LOOPS", "1"))


class Runner:
    def __init__(self, add_cmdline_args=None, **kwargs):
        self.metadata = {}
        self.argparser = argparse.ArgumentParser()
        self._args = None

    def parse_args(self, args=None):
        if self._args is None:
            self._args = self.argparser.parse_args(args)
        return self._args

    def bench_func(self, name, func, *args):
        for _ in range(LOOPS):
            result = func(*args)
        sys.stderr.write("perf_ci: %s ok (%d loops)\n" % (name, LOOPS))
        return result

    def bench_time_func(self, name, time_func, *args):
        result = time_func(LOOPS, *args)
        sys.stderr.write("perf_ci: %s ok (%d loops)\n" % (name, LOOPS))
        return result
