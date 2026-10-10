import _sitebuiltins
import io
import sys

text = "\n".join(f"license line {number}" for number in range(26))
printer = _sitebuiltins._Printer("license", text)
streams = sys.stdin, sys.stdout, sys.stderr
stdin, stdout, stderr = io.StringIO(), io.StringIO(), io.StringIO()
try:
    sys.stdin, sys.stdout, sys.stderr = stdin, stdout, stderr
    printer()
finally:
    sys.stdin, sys.stdout, sys.stderr = streams

assert stdout.getvalue() == text
assert stderr.getvalue() == ""
