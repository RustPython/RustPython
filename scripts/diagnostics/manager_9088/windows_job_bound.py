"""Source-review-only proposal: no execution until the bounded scope is approved."""

import json
import os
import pathlib
import subprocess
import sys
import time

if sys.platform != "win32" or sys.implementation.name != "rustpython":
    raise SystemExit("This diagnostic requires the reviewed Windows RustPython APIs")
if not sys.flags.ignore_environment or not sys.flags.no_site:
    raise SystemExit(
        "Launch the supervisor with -E -S to exclude ambient startup configuration"
    )
import _winapi

plans = {
    "barrier-abort": "manager_barrier_abort.py",
    "pool-ordinary": "manager_pool_ordinary.py",
}
if len(sys.argv) != 2 or sys.argv[1] not in plans:
    raise SystemExit("Choose one reviewed diagnostic plan")
plan = sys.argv[1]
fixture = pathlib.Path(__file__).with_name(plans[plan]).resolve()
log_path = pathlib.Path(__file__).with_name(plan + ".log")
result_path = pathlib.Path(__file__).with_name(plan + ".json")
bootstrap = (
    "import sys\n"
    "token = sys.stdin.buffer.read(3)\n"
    "sys.stdin.close()\n"
    "if token != b'go\\n':\n"
    "    raise SystemExit(2)\n"
    "import runpy\n"
    "sys.argv = [sys.argv[1]]\n"
    "runpy.run_path(sys.argv[0], run_name='__main__')\n"
)
command = [sys.executable, "-E", "-S", "-B", "-u", "-c", bootstrap, str(fixture)]
result = {
    "plan": plan,
    "command": command,
    "timeout_seconds": 90,
    "admitted": False,
    "status": "error",
    "exit": 1,
    "timed_out": False,
    "cleanup_errors": [],
}
job = None
child = None
started = time.monotonic()
try:
    job = _winapi.CreateJobObject(None)
    _winapi.SetJobObjectKillOnClose(job)
    env = os.environ.copy()
    for name in ("PYTHONPATH", "PYTHONFAULTHANDLER", "PYTHONOPTIMIZE", "PYTHONINSPECT"):
        env.pop(name, None)
    env.update(PYTHONDONTWRITEBYTECODE="1", PYTHONUNBUFFERED="1")
    with log_path.open("wb") as log:
        child = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=log,
            stderr=subprocess.STDOUT,
            close_fds=True,
            env=env,
        )
        # The bootstrap cannot import/run the fixture or spawn descendants
        # until assignment succeeds and the exact admission token is received.
        _winapi.AssignProcessToJobObject(job, int(child._handle))
        result["admitted"] = True
        child.stdin.write(b"go\n")
        child.stdin.flush()
        child.stdin.close()
        try:
            returncode = child.wait(timeout=max(0.0, 90 - (time.monotonic() - started)))
        except subprocess.TimeoutExpired:
            result.update(status="timeout", exit=124, timed_out=True)
            _winapi.TerminateJobObject(job, 124)
            child.wait(timeout=5)
        else:
            result.update(
                status="passed" if returncode == 0 else "failed", exit=returncode
            )
except BaseException as error:
    result.update(status="error", exit=1, error=repr(error))
finally:
    # No unassigned child is ever admitted. On assignment failure it remains
    # blocked before fixture imports and is killed directly, with no fallback run.
    if child is not None and not result["admitted"]:
        try:
            child.kill()
            child.wait(timeout=5)
        except BaseException as error:
            result["cleanup_errors"].append(repr(error))
    if job is not None:
        try:
            # Also cleans any descendants left after a normal test exit.
            _winapi.TerminateJobObject(job, 1)
        except BaseException as error:
            result["cleanup_errors"].append(repr(error))
        try:
            _winapi.CloseHandle(job)
        except BaseException as error:
            result["cleanup_errors"].append(repr(error))
    if child is not None:
        try:
            if child.stdin is not None:
                child.stdin.close()
            child.wait(timeout=5)
        except BaseException as error:
            result["cleanup_errors"].append(repr(error))
    if result["cleanup_errors"]:
        result.update(status="cleanup_error", exit=1)
    result["seconds"] = round(time.monotonic() - started, 2)
    result_path.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result), flush=True)
    if log_path.exists():
        with log_path.open("rb") as captured:
            print(
                captured.read(65536).decode("utf-8", errors="backslashreplace"),
                flush=True,
            )
raise SystemExit(result["exit"])
