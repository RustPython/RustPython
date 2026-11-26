"""Check keyword acceptance that inspect.signature cannot always observe."""

import json
import os
import subprocess
import sys
from pathlib import Path

# Do not raise any signals. Registration and enable/disable take place in an
# isolated child process; the probe leaves no handlers installed.
PROBE = """
import faulthandler
import json
import signal

result = {}
def check(name, call, reset):
    try:
        call()
    except Exception as exc:
        result[name] = type(exc).__name__
    else:
        reset()
        result[name] = "accepted"

check("enable.c_stack", lambda: faulthandler.enable(c_stack=False),
      faulthandler.disable)
check("enable.max_threads", lambda: faulthandler.enable(max_threads=1),
      faulthandler.disable)
if hasattr(faulthandler, "register") and hasattr(signal, "SIGUSR1"):
    check("register.max_threads",
          lambda: faulthandler.register(signal.SIGUSR1, max_threads=1),
          lambda: faulthandler.unregister(signal.SIGUSR1))
print(json.dumps(result))
"""


def run(executable):
    result = subprocess.run(
        [executable, "-I", "-c", PROBE],
        check=True,
        capture_output=True,
        text=True,
        timeout=25,
        env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"},
    )
    return json.loads(result.stdout)


def main():
    executable = Path(sys.argv[1]).resolve()
    if os.name == "nt" and not executable.suffix:
        executable = executable.with_suffix(".exe")
    expected, actual = run(sys.executable), run(executable)
    print(json.dumps({"cpython": expected, "rustpython": actual}, indent=2))
    if actual != expected:
        print("faulthandler keyword behavior differs from CPython", file=sys.stderr)
    return actual != expected


if __name__ == "__main__":
    raise SystemExit(main())
