import platform
import sys

def assert_raises(expected, *args, _msg=None, **kw):
    if args:
        f, f_args = args[0], args[1:]
        with AssertRaises(expected, _msg):
            f(*f_args, **kw)
    else:
        assert not kw
        return AssertRaises(expected, _msg)


class AssertRaises:
    def __init__(self, expected, msg):
        self.expected = expected
        self.exception = None
        self.failmsg = msg

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        if exc_type is None:
            failmsg = self.failmsg or \
                '{} was not raised'.format(self.expected.__name__)
            assert False, failmsg
        if not issubclass(exc_type, self.expected):
            return False

        self.exception = exc_val
        return True


class TestFailingBool:
    def __bool__(self):
        raise RuntimeError

class TestFailingIter:
    def __iter__(self):
        raise RuntimeError


def _assert_print(f, args):
    raised = True
    try:
        assert f()
        raised = False
    finally:
        if raised:
            print('Assertion Failure:', *args)

def _typed(obj):
    return '{}({})'.format(type(obj), obj)


def assert_equal(a, b):
    _assert_print(lambda: a == b, [_typed(a), '==', _typed(b)])


def assert_true(e):
    _assert_print(lambda: e is True, [_typed(e), 'is True'])


def assert_false(e):
    _assert_print(lambda: e is False, [_typed(e), 'is False'])

def assert_isinstance(obj, klass):
    _assert_print(lambda: isinstance(obj, klass), ['isisntance(', _typed(obj), ',', klass, ')'])

def assert_in(a, b):
    _assert_print(lambda: a in b, [a, 'in', b])

def skip_if_unsupported(req_maj_vers, req_min_vers, test_fct):
    def exec():
        test_fct()

    if platform.python_implementation() == 'RustPython':
        exec()
    elif sys.version_info.major>=req_maj_vers and sys.version_info.minor>=req_min_vers:
        exec()
    else:
        print(f'Skipping test as a higher python version is required. Using {platform.python_implementation()} {platform.python_version()}')

def fail_if_unsupported(req_maj_vers, req_min_vers, test_fct):
    def exec():
        test_fct()

    if platform.python_implementation() == 'RustPython':
        exec()
    elif sys.version_info.major>=req_maj_vers and sys.version_info.minor>=req_min_vers:
        exec()
    else:
        assert False, f'Test cannot performed on this python version. {platform.python_implementation()} {platform.python_version()}'
