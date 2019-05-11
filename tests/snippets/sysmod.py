import sys

print(sys.argv)
assert sys.argv[0].endswith('.py')

assert sys.platform == "linux" or sys.platform == "darwin" or sys.platform == "win32" or sys.platform == "unknown"
