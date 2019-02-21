
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

import compile_code


class _TestType(enum.Enum):
    functional = 1
    benchmark = 2


logger = logging.getLogger('tests')
ROOT_DIR = '..'
TEST_ROOT = os.path.abspath(os.path.join(ROOT_DIR, 'tests'))
TEST_DIRS = {
    _TestType.functional: os.path.join(TEST_ROOT, 'snippets'),
    _TestType.benchmark: os.path.join(TEST_ROOT, 'benchmarks'),
}
CPYTHON_RUNNER_DIR = os.path.abspath(os.path.join(ROOT_DIR, 'py_code_object'))
RUSTPYTHON_RUNNER_DIR = os.path.abspath(os.path.join(ROOT_DIR))


@contextlib.contextmanager
def pushd(path):
    old_dir = os.getcwd()
    os.chdir(path)
    yield
    os.chdir(old_dir)


def perform_test(filename, method, test_type):
    logger.info('Running %s via %s', filename, method)
    if method == 'cpython':
        run_via_cpython(filename)
    elif method == 'cpython_bytecode':
        run_via_cpython_bytecode(filename, test_type)
    elif method == 'rustpython':
        run_via_rustpython(filename, test_type)
    else:
        raise NotImplementedError(method)


def run_via_cpython(filename):
    """ Simply invoke python itself on the script """
    env = os.environ.copy()
    subprocess.check_call([sys.executable, filename], env=env)


def run_via_cpython_bytecode(filename, test_type):
    # Step1: Create bytecode file:
    bytecode_filename = filename + '.bytecode'
    with open(bytecode_filename, 'w') as f:
        compile_code.compile_to_bytecode(filename, out_file=f)

    # Step2: run cpython bytecode:
    env = os.environ.copy()
    log_level = 'info' if test_type == _TestType.benchmark else 'debug'
    env['RUST_LOG'] = '{},cargo=error,jobserver=error'.format(log_level)
    env['RUST_BACKTRACE'] = '1'
    with pushd(CPYTHON_RUNNER_DIR):
        subprocess.check_call(['cargo', 'run', bytecode_filename], env=env)


def run_via_rustpython(filename, test_type):
    env = os.environ.copy()
    log_level = 'info' if test_type == _TestType.benchmark else 'trace'
    env['RUST_LOG'] = '{},cargo=error,jobserver=error'.format(log_level)
    env['RUST_BACKTRACE'] = '1'
    if env.get('CODE_COVERAGE', 'false') == 'true':
        subprocess.check_call(
            ['cargo', 'run', filename], env=env)
    else:
        subprocess.check_call(
            ['cargo', 'run', '--release', filename], env=env)


def create_test_function(cls, filename, method, test_type):
    """ Create a test function for a single snippet """
    core_test_directory, snippet_filename = os.path.split(filename)
    test_function_name = 'test_{}_'.format(method) \
        + os.path.splitext(snippet_filename)[0] \
        .replace('.', '_').replace('-', '_')

    def test_function(self):
        perform_test(filename, method, test_type)

    if hasattr(cls, test_function_name):
        raise ValueError('Duplicate test case {}'.format(test_function_name))
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
        for filepath in sorted(glob.iglob(os.path.join(test_dir, '*.py'))):
            filename = os.path.split(filepath)[1]
            if filename.startswith('xfail_'):
                continue

            yield test_type, os.path.abspath(filepath)


@populate('cpython')
# @populate('cpython_bytecode')
@populate('rustpython')
class SampleTestCase(unittest.TestCase):
    pass
