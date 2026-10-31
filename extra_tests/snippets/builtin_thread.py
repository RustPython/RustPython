import _thread

assert _thread.TIMEOUT_MAX in [9223372036.0, 4294967.0]

from testutils import assert_raises

# Reentrant calls must validate arguments before incrementing the count.
for error, kwargs in (
    (ValueError, {"blocking": False, "timeout": 1}),
    (ValueError, {"timeout": -2}),
    (OverflowError, {"timeout": _thread.TIMEOUT_MAX + 1}),
    (OverflowError, {"blocking": False, "timeout": 2**63 / 1e9}),
    (OverflowError, {"blocking": False, "timeout": -9_223_372_037}),
    (OverflowError, {"timeout": -9_223_372_037}),
    (OverflowError, {"blocking": False, "timeout": float("inf")}),
    (OverflowError, {"timeout": float("inf")}),
    (OverflowError, {"blocking": False, "timeout": float("-inf")}),
    (OverflowError, {"timeout": float("-inf")}),
    (ValueError, {"timeout": -(2**63) / 1e9}),
    (ValueError, {"blocking": False, "timeout": 4_294_968}),
    (ValueError, {"timeout": -4_294_968}),
):
    lock = _thread.RLock()
    assert lock.acquire()
    try:
        with assert_raises(error):
            lock.acquire(**kwargs)
        assert lock._recursion_count() == 1
    finally:
        while lock._is_owned():
            lock.release()
