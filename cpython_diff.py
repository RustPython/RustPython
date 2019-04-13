import difflib
import glob
import os
import sys


RUSTPYTHON_ROOT = os.path.dirname(__file__)
try:
    # This should point to the root the the cpython git repository
    CPYTHON_ROOT = os.environ["CPYTHON_ROOT"]
except KeyError:
    print("Please define CPYTHON_ROOT environment variable", file=sys.stderr)
    CPYTHON_ROOT = None


def compare_files(rustpython_path, cpython_path):
    rustpython_lines = open(rustpython_path).readlines()
    cpython_lines = open(cpython_path).readlines()

    diff_lines = difflib.unified_diff(cpython_lines, rustpython_lines,
                                      fromfile=cpython_path, tofile=rustpython_path)
    print(*diff_lines, sep='', end='')


def walk_directory(rustpython_base, cpython_base):
    rustpython_base = os.path.join(RUSTPYTHON_ROOT, rustpython_base)
    cpython_base = os.path.join(CPYTHON_ROOT, cpython_base)
    for rustpython_path in glob.glob(os.path.join(rustpython_base, "**", "*.py"), recursive=True):
        rel_path = os.path.relpath(rustpython_path, rustpython_base)
        cpython_path = os.path.join(cpython_base, rel_path)
        if os.path.exists(cpython_path):
            compare_files(rustpython_path, cpython_path)


def main():
    if CPYTHON_ROOT is not None:
        walk_directory("Lib", "Lib")
        walk_directory(os.path.join("tests", 'cpython_tests'),
                       os.path.join("Lib", 'test'))


if __name__ == '__main__':
    main()
