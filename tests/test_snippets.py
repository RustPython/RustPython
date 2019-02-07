
# This is a python unittest class automatically populating with all tests
# in the tests folder.


import sys
import os
import logging
import subprocess
import contextlib
import enum
import pytest
from pathlib import Path

import compile_code


class _TestType(enum.Enum):
    functional = 1
    benchmark = 2


logger = logging.getLogger('tests')
ROOT_DIR = Path('..').absolute()
TEST_ROOT = ROOT_DIR / 'tests'
TEST_DIRS = {
    _TestType.functional: TEST_ROOT / 'snippets',
    _TestType.benchmark: TEST_ROOT / 'benchmarks',
}
CPYTHON_RUNNER_DIR = ROOT_DIR / 'py_code_object'
RUSTPYTHON_RUNNER_DIR = ROOT_DIR


@contextlib.contextmanager
def pushd(path):
    old_dir = os.getcwd()
    os.chdir(path)
    yield
    os.chdir(old_dir)


def get_test_files():
    """ Retrieve test files """
    for test_type, test_dir in TEST_DIRS.items():
        for filepath in sorted(test_dir.glob('*.py')):
            if filepath.name.startswith('xfail_'):
                continue

            yield test_type, filepath


def run_rust_python(test_type, filename):
    env = os.environ.copy()
    log_level = 'info' if test_type == _TestType.benchmark else 'debug'
    env['RUST_LOG'] = '{},cargo=error,jobserver=error'.format(log_level)
    env['RUST_BACKTRACE'] = '1'
    with pushd(CPYTHON_RUNNER_DIR):
        subprocess.check_call(['cargo', 'run', filename], env=env)


@pytest.mark.parametrize("test_type, filename", get_test_files())
def test_cpython(test_type, filename):
    env = os.environ.copy()
    subprocess.check_call([sys.executable, filename], env=env)


@pytest.mark.parametrize("test_type, filename", get_test_files())
def test_rustpython(test_type, filename):
    run_rust_python(test_type, filename)


@pytest.mark.parametrize("test_type, filename", get_test_files())
@pytest.mark.skip(reason="Currently non-functional")
def test_rustpython_bytecode(test_type, filename, tmpdir):
    bytecode_filename = tmpdir.join(filename.with_suffix('.bytecode'))
    with open(bytecode_filename, 'w') as f:
        compile_code.compile_to_bytecode(filename, out_file=f)

    # Step2: run cpython bytecode:
    run_rust_python(test_type, bytecode_filename)

