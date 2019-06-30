import sys

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
