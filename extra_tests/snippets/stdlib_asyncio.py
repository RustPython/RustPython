"""The private _asyncio accessors, reached directly instead of through a loop.

CPython's _asyncio rejects every call below with "loop ... is not the running
loop" before it gets anywhere, and does not expose _current_tasks at all, so
these only run where they are reachable.
"""

import subprocess
import sys

from testutils import assert_raises

if sys.platform == "win32":
    # An unclosed proactor loop must not deadlock during interpreter shutdown.
    subprocess.run(
        [
            sys.executable,
            "-c",
            "import asyncio; loop = asyncio.new_event_loop(); "
            "asyncio.set_event_loop(loop); loop.run_until_complete(asyncio.sleep(0))",
        ],
        check=True,
        timeout=10,
    )

if sys.implementation.name != "rustpython":
    sys.exit(0)

import _asyncio


def _task():
    pass


# The "already entered" message formats both tasks; a plain function used to be
# formatted as the wrong type there.
_asyncio._enter_task(0, _task)
with assert_raises(RuntimeError) as cm:
    _asyncio._enter_task(0, _task)
assert "Cannot enter into task" in str(cm.exception), cm.exception
assert "<function _task at " in str(cm.exception), cm.exception
_asyncio._leave_task(0, _task)

# The module-level task registry is writable from Python, so current_task() has
# to cope with it holding something that is not a mapping.
saved_current_tasks = _asyncio._current_tasks
try:
    _asyncio._current_tasks = 42
    assert _asyncio.current_task(loop=object()) is None
finally:
    _asyncio._current_tasks = saved_current_tasks


class BadException(Exception):
    def __new__(cls, *args):
        return 42


# throw() has to check what __new__ handed back before treating it as an
# exception.
future = _asyncio.Future(loop=object())
with assert_raises(TypeError):
    future.__await__().throw(BadException)

# InvalidStateError is looked up in asyncio.exceptions every time a pending
# future is asked for its result, so what is found there need not be an
# exception type at all.
import asyncio  # noqa: E402
import asyncio.exceptions  # noqa: E402

pending = _asyncio.Future(loop=object())
with assert_raises(asyncio.exceptions.InvalidStateError):
    pending.result()

saved_invalid_state_error = asyncio.exceptions.InvalidStateError
try:
    for replacement in (lambda *args: 42, None):
        asyncio.InvalidStateError = replacement
        asyncio.exceptions.InvalidStateError = replacement
        with assert_raises(RuntimeError):
            pending.result()
        with assert_raises(RuntimeError):
            pending.exception()
finally:
    asyncio.InvalidStateError = saved_invalid_state_error
    asyncio.exceptions.InvalidStateError = saved_invalid_state_error

# The awaited-by set is built with the waiter's __hash__, which can come back
# to the same future; the field must not be locked while that runs.


class Reentrant(_asyncio.Future):
    def __hash__(self):
        _asyncio.future_add_to_awaited_by(awaited, Reentrant(loop=object()))
        return 1

    def __eq__(self, other):
        return self is other


awaited = _asyncio.Future(loop=object())
_asyncio.future_add_to_awaited_by(awaited, Reentrant(loop=object()))
with assert_raises(RecursionError):
    # converting the single waiter into a set hashes both of them
    _asyncio.future_add_to_awaited_by(awaited, Reentrant(loop=object()))

plain = _asyncio.Future(loop=object())
waiter = _asyncio.Future(loop=object())
_asyncio.future_add_to_awaited_by(plain, waiter)
_asyncio.future_add_to_awaited_by(plain, _asyncio.Future(loop=object()))
assert waiter in plain._asyncio_awaited_by
_asyncio.future_discard_from_awaited_by(plain, waiter)
assert waiter not in plain._asyncio_awaited_by

print("ok")
