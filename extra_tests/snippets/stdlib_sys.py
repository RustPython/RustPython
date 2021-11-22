import sys

from testutils import assert_raises

print('python executable:', sys.executable)
print(sys.argv)
assert sys.argv[0].endswith('.py')

assert sys.platform == "linux" or sys.platform == "darwin" or sys.platform == "win32" or sys.platform == "unknown"

if hasattr(sys, "_framework"):
    assert type(sys._framework) is str

assert isinstance(sys.builtin_module_names, tuple)
assert 'sys' in sys.builtin_module_names

assert isinstance(sys.implementation.name, str)
assert isinstance(sys.implementation.cache_tag, str)

assert sys.getfilesystemencoding() == 'utf-8'
assert sys.getfilesystemencodeerrors().startswith('surrogate')

assert sys.byteorder == "little" or sys.byteorder == "big"

assert isinstance(sys.flags, tuple)
assert type(sys.flags).__name__ == "flags"
assert type(sys.flags.optimize) is int
assert sys.flags[3] == sys.flags.optimize
assert sys.maxunicode == 1114111


# Tracing:

events = []

def trc(frame, event, arg):
    fn_name = frame.f_code.co_name
    events.append((fn_name, event, arg))
    print('trace event:', fn_name, event, arg)

def demo(x):
    if x > 0:
        demo(x - 1)

sys.settrace(trc)
demo(5)
sys.settrace(None)

assert ("demo", "call", None) in events

assert sys.exc_info() == (None, None, None)

try:
    1/0
except ZeroDivisionError as exc:
    exc_info = sys.exc_info()
    assert exc_info[0] == type(exc) == ZeroDivisionError
    assert exc_info[1] == exc


# Recursion:

def recursive_call(n):
    if n > 0:
        recursive_call(n - 1)

sys.setrecursionlimit(200)
assert sys.getrecursionlimit() == 200

with assert_raises(RecursionError):
    recursive_call(300)

if sys.platform.startswith("win"):
    winver = sys.getwindowsversion()
    print(f'winver: {winver} {winver.platform_version}')

    # the biggest value of wSuiteMask (https://docs.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-osversioninfoexa#members).
    all_masks = 0x00000004 | 0x00000400 | 0x00004000 | 0x00000080 | 0x00000002 | 0x00000040 | 0x00000200 | \
        0x00000100 | 0x00000001 | 0x00000020 | 0x00002000 | 0x00000010 | 0x00008000 | 0x00020000

    # We really can't test if the results are correct, so it just checks for meaningful value
    assert winver.major > 0
    assert winver.minor >= 0
    assert winver.build > 0
    assert winver.platform == 2
    assert isinstance(winver.service_pack, str)
    assert 0 <= winver.suite_mask <= all_masks
    assert 1 <= winver.product_type <= 3

    # XXX if platform_version is implemented correctly, this'll break on compatiblity mode or a build without manifest
    # these fields can mismatch in CPython
    # assert winver.major == winver.platform_version[0]
    # assert winver.minor == winver.platform_version[1]
    # assert winver.build == winver.platform_version[2]
