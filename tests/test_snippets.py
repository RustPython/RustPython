
# This is a python unittest class automatically populating with all tests
# in the tests folder.


import sys
import os
import unittest
import glob
import logging
import subprocess
import contextlib

import compile_code


logger = logging.getLogger('tests')
ROOT_DIR = '..'
TEST_DIR = os.path.abspath(os.path.join(ROOT_DIR, 'tests', 'snippets'))
CPYTHON_RUNNER_DIR = os.path.abspath(os.path.join(ROOT_DIR, 'py_code_object'))
RUSTPYTHON_RUNNER_DIR = os.path.abspath(os.path.join(ROOT_DIR))


@contextlib.contextmanager
def pushd(path):
    old_dir = os.getcwd()
    os.chdir(path)
    yield
    os.chdir(old_dir)


def perform_test(filename, method):
    logger.info('Running %s via %s', filename, method)
    if method == 'cpython':
        run_via_cpython(filename)
    elif method == 'cpython_bytecode':
        run_via_cpython_bytecode(filename)
    elif method == 'rustpython':
        run_via_rustpython(filename)
    else:
        raise NotImplementedError(method)


def run_via_cpython(filename):
    """ Simply invoke python itself on the script """
    subprocess.check_call([sys.executable, filename])


def run_via_cpython_bytecode(filename):
    # Step1: Create bytecode file:
    bytecode_filename = filename + '.bytecode'
    with open(bytecode_filename, 'w') as f:
        compile_code.compile_to_bytecode(filename, out_file=f)

    # Step2: run cpython bytecode:
    env = os.environ.copy()
    env['RUST_LOG'] = 'debug'
    env['RUST_BACKTRACE'] = '1'
    with pushd(CPYTHON_RUNNER_DIR):
        subprocess.check_call(['cargo', 'run', bytecode_filename], env=env)


def run_via_rustpython(filename):
    env = os.environ.copy()
    env['RUST_LOG'] = 'trace'
    env['RUST_BACKTRACE'] = '1'
    with pushd(RUSTPYTHON_RUNNER_DIR):
        subprocess.check_call(['cargo', 'run', filename], env=env)


def create_test_function(cls, filename, method):
    """ Create a test function for a single snippet """
    core_test_directory, snippet_filename = os.path.split(filename)
    test_function_name = 'test_{}_'.format(method) \
        + os.path.splitext(snippet_filename)[0] \
        .replace('.', '_').replace('-', '_')

    def test_function(self):
        perform_test(filename, method)

    if hasattr(cls, test_function_name):
        raise ValueError('Duplicate test case {}'.format(test_function_name))
    setattr(cls, test_function_name, test_function)


def populate(cls):
    """ Decorator function which can populate a unittest.TestCase class """
    for method in ['cpython', 'cpython_bytecode', 'rustpython']:
        for filename in get_test_files():
            create_test_function(cls, filename, method)
    return cls


def get_test_files():
    """ Retrieve test files """
    for filepath in sorted(glob.iglob(os.path.join(
            TEST_DIR, '*.py'))):
        filename = os.path.split(filepath)[1]
        if filename.startswith('xfail_'):
            continue

        yield os.path.abspath(filepath)


@populate
class SampleTestCase(unittest.TestCase):
    pass
