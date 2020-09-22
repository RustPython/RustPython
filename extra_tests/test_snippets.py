# This is a python unittest class automatically populating with all tests
# in the tests folder.


import sys
import os
import unittest
import glob
import logging
import subprocess
import contextlib
import enum
from pathlib import Path
import shutil


class _TestType(enum.Enum):
    functional = 1


logger = logging.getLogger("tests")
ROOT_DIR = ".."
TEST_ROOT = os.path.abspath(os.path.join(ROOT_DIR, "extra_tests"))
TEST_DIRS = {_TestType.functional: os.path.join(TEST_ROOT, "snippets")}
CPYTHON_RUNNER_DIR = os.path.abspath(os.path.join(ROOT_DIR, "py_code_object"))
RUSTPYTHON_RUNNER_DIR = os.path.abspath(os.path.join(ROOT_DIR))
RUSTPYTHON_LIB_DIR = os.path.abspath(os.path.join(ROOT_DIR, "Lib"))
RUSTPYTHON_FEATURES = ["jit"]


@contextlib.contextmanager
def pushd(path):
    old_dir = os.getcwd()
    os.chdir(path)
    yield
    os.chdir(old_dir)


def perform_test(filename, method, test_type):
    logger.info("Running %s via %s", filename, method)
    if method == "cpython":
        run_via_cpython(filename)
    elif method == "rustpython":
        run_via_rustpython(filename, test_type)
    else:
        raise NotImplementedError(method)


def run_via_cpython(filename):
    """ Simply invoke python itself on the script """
    env = os.environ.copy()
    subprocess.check_call([sys.executable, filename], env=env)

SKIP_BUILD = os.environ.get("RUSTPYTHON_TESTS_NOBUILD") == "true"
RUST_DEBUG = os.environ.get("RUSTPYTHON_DEBUG") == "true"
RUST_PROFILE = "debug" if RUST_DEBUG else "release"

def run_via_rustpython(filename, test_type):
    env = os.environ.copy()
    env['RUST_LOG'] = 'info,cargo=error,jobserver=error'
    env['RUST_BACKTRACE'] = '1'
    env['PYTHONPATH'] = RUSTPYTHON_LIB_DIR

    binary = os.path.abspath(os.path.join(ROOT_DIR, "target", RUST_PROFILE, "rustpython"))

    subprocess.check_call([binary, filename], env=env)


def create_test_function(cls, filename, method, test_type):
    """ Create a test function for a single snippet """
    core_test_directory, snippet_filename = os.path.split(filename)
    test_function_name = "test_{}_".format(method) + os.path.splitext(snippet_filename)[
        0
    ].replace(".", "_").replace("-", "_")

    def test_function(self):
        perform_test(filename, method, test_type)

    if hasattr(cls, test_function_name):
        raise ValueError("Duplicate test case {}".format(test_function_name))
    setattr(cls, test_function_name, test_function)


def populate(method):
    def wrapper(cls):
        """ Decorator function which can populate a unittest.TestCase class """
        for test_type, filename in get_test_files():
            create_test_function(cls, filename, method, test_type)
        return cls

    return wrapper


def get_test_files():
    """ Retrieve test files """
    for test_type, test_dir in TEST_DIRS.items():
        for filepath in sorted(glob.iglob(os.path.join(test_dir, "*.py"))):
            filename = os.path.split(filepath)[1]
            if filename.startswith("xfail_"):
                continue

            yield test_type, os.path.abspath(filepath)


def generate_slices(path):
    # loop used to build slices_res.py with cpython
    ll = [0, 1, 2, 3]
    start = list(range(-7, 7))
    end = list(range(-7, 7))
    step = list(range(-5, 5))
    step.pop(step.index(0))
    for i in [start, end, step]:
        i.append(None)

    slices_res = []
    for s in start:
        for e in end:
            for t in step:
                slices_res.append(ll[s:e:t])

    path.write_text(
        "SLICES_RES={}\nSTART= {}\nEND= {}\nSTEP= {}\nLL={}\n".format(
            slices_res, start, end, step, ll
        )
    )


@populate("cpython")
@populate("rustpython")
class SampleTestCase(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Here add resource files
        cls.slices_resource_path = Path(TEST_DIRS[_TestType.functional]) / "cpython_generated_slices.py"
        if cls.slices_resource_path.exists():
            cls.slices_resource_path.unlink()

        generate_slices(cls.slices_resource_path)

        if not SKIP_BUILD:
            # cargo stuff
            profile_args = [] if RUST_DEBUG else ["--release"]
            subprocess.check_call(["cargo", "build", "--features", ",".join(RUSTPYTHON_FEATURES), *profile_args])

    @classmethod
    def tearDownClass(cls):
        cls.slices_resource_path.unlink()
