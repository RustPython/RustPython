
import time
import sys

import pytest
import subprocess

from benchmarks import nbody

# Interpreters:
rustpython_exe = '../target/release/rustpython'
cpython_exe = sys.executable
pythons = [
    cpython_exe,
    rustpython_exe
]

# Benchmark scripts:
benchmarks = [
    ['benchmarks/nbody.py'],
    ['benchmarks/mandelbrot.py'],
]

@pytest.mark.parametrize('exe', pythons)
@pytest.mark.parametrize('args', benchmarks)
def test_bench(exe, args, benchmark):
    def bench():
        subprocess.run([exe] + args)

    benchmark(bench)

