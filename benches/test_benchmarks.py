import os
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
BENCHES = ROOT / "benches"

# Interpreters:
default_rustpython = (
    ROOT
    / "target"
    / "release"
    / ("rustpython.exe" if os.name == "nt" else "rustpython")
)
rustpython_exe = str(Path(os.environ.get("RUSTPYTHON", default_rustpython)).resolve())
cpython_exe = sys.executable
pythons = [cpython_exe, rustpython_exe]

# Benchmark scripts:
benchmarks = [
    BENCHES / "benchmarks" / "nbody.py",
    BENCHES / "benchmarks" / "mandelbrot.py",
    BENCHES / "microbenchmarks" / "strings.py",
]

exe_ids = ["cpython", "rustpython"]
benchmark_ids = [benchmark.name for benchmark in benchmarks]


@pytest.mark.parametrize("exe", pythons, ids=exe_ids)
@pytest.mark.parametrize("script", benchmarks, ids=benchmark_ids)
def test_bench(exe, script, benchmark):
    benchmark(subprocess.run, [exe, str(script)], cwd=ROOT, check=True)
