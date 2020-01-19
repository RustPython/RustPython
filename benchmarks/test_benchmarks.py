
import os
import time
import sys

import pytest
import subprocess

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
    ['benchmarks/strings.py'],
]

exe_ids = ['cpython', 'rustpython']
benchmark_ids = [benchmark[0].split('/')[-1] for benchmark in benchmarks]

@pytest.mark.parametrize('exe', pythons, ids=exe_ids)
@pytest.mark.parametrize('args', benchmarks, ids=benchmark_ids)
def test_bench(exe, args, benchmark):
    def bench():
        subprocess.run([exe] + args)

    benchmark(bench)

