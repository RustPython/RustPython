"""The private _asyncio accessors, reached directly instead of through a loop.

CPython's _asyncio rejects every call below with "loop ... is not the running
loop" before it gets anywhere, and does not expose _current_tasks at all, so
these only run where they are reachable.
"""

import sys

from testutils import assert_raises

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

print("ok")
