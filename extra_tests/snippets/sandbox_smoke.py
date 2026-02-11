"""Sandbox mode smoke test.

Verifies basic functionality that works in both sandbox and normal mode:
- stdio (print, sys.stdout/stdin/stderr)
- builtin modules (math, json)
- in-memory IO (BytesIO, StringIO)
- open() is properly blocked when FileIO is unavailable (sandbox)
"""

import _io
import json
import math
import sys

SANDBOX = not hasattr(_io, "FileIO")

# stdio
print("1. print works")
assert sys.stdout.writable()
assert sys.stderr.writable()
assert sys.stdin.readable()
assert sys.stdout.fileno() == 1

# math
assert math.pi > 3.14
print("2. math works:", math.pi)

# json
d = json.loads('{"a": 1}')
assert d == {"a": 1}
print("3. json works:", d)

# BytesIO / StringIO
buf = _io.BytesIO(b"hello")
assert buf.read() == b"hello"
sio = _io.StringIO("world")
assert sio.read() == "world"
print("4. BytesIO/StringIO work")

# open() behavior depends on mode
if SANDBOX:
    try:
        open("/tmp/x", "w")
        assert False, "should have raised"
    except _io.UnsupportedOperation:
        print("5. open() properly blocked (sandbox)")
else:
    print("5. open() available (host_env)")

# builtins
assert list(range(5)) == [0, 1, 2, 3, 4]
assert sorted([3, 1, 2]) == [1, 2, 3]
print("6. builtins work")

print("All smoke tests passed!", "(sandbox)" if SANDBOX else "(host_env)")
