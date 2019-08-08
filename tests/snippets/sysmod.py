import sys

print('python executable:', sys.executable)
print(sys.argv)
assert sys.argv[0].endswith('.py')

assert sys.platform == "linux" or sys.platform == "darwin" or sys.platform == "win32" or sys.platform == "unknown"

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

def trc(frame, event, arg):
    print('trace event:', frame, event, arg)

def demo(x):
    print(x)
    if x > 0:
        demo(x - 1)

sys.settrace(trc)
demo(5)
sys.settrace(None)

assert sys.exc_info() == (None, None, None)

try:
	1/0
except ZeroDivisionError as exc:
	exc_info = sys.exc_info()
	assert exc_info[0] == type(exc) == ZeroDivisionError
	assert exc_info[1] == exc
