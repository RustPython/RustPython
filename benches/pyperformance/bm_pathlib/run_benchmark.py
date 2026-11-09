"""
Test the performance of pathlib operations.

This benchmark stresses the creation of small objects, globbing, and system
calls.
"""

# Python imports
import os
import pathlib
import shutil
import tempfile

import pyperf


NUM_FILES = 2000


def generate_filenames(tmp_path, num_files):
    i = 0
    while num_files:
        for ext in [".py", ".txt", ".tar.gz", ""]:
            i += 1
            yield os.path.join(tmp_path, str(i) + ext)
            num_files -= 1


def setup(num_files):
    tmp_path = tempfile.mkdtemp()
    for fn in generate_filenames(tmp_path, num_files):
        with open(fn, "wb") as f:
            f.write(b'benchmark')

    return tmp_path


def bench_pathlib(loops, tmp_path):
    base_path = pathlib.Path(tmp_path)

    # Warm up the filesystem cache and keep some objects in memory.
    path_objects = list(base_path.iterdir())
    # FIXME: does this code really cache anything?
    for p in path_objects:
        p.stat()
    assert len(path_objects) == NUM_FILES, len(path_objects)

    range_it = range(loops)
    t0 = pyperf.perf_counter()

    for _ in range_it:
        # Do something simple with each path.
        for p in base_path.iterdir():
            p.stat()
        for p in base_path.glob("*.py"):
            p.stat()
        for p in base_path.iterdir():
            p.stat()
        for p in base_path.glob("*.py"):
            p.stat()

    return pyperf.perf_counter() - t0


if __name__ == "__main__":
    runner = pyperf.Runner()
    runner.metadata['description'] = ("Test the performance of "
                                      "pathlib operations.")

    modname = pathlib.__name__
    runner.metadata['pathlib_module'] = modname

    tmp_path = setup(NUM_FILES)
    try:
        runner.bench_time_func('pathlib', bench_pathlib, tmp_path)
    finally:
        shutil.rmtree(tmp_path)
