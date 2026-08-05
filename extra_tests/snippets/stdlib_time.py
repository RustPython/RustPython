import sys
import time

x = time.gmtime(1000)

assert x.tm_year == 1970
assert x.tm_min == 16
assert x.tm_sec == 40
assert x.tm_isdst == 0

s = time.strftime("%Y-%m-%d-%H-%M-%S", x)
# print(s)
assert s == "1970-01-01-00-16-40"

if sys.platform != "wasi":
    # _strptime depends on time.tzname, which is not available on WASI yet.
    x2 = time.strptime(s, "%Y-%m-%d-%H-%M-%S")
    assert x2.tm_min == 16

    # TODO: WASI currently does not raise OverflowError for some out-of-range
    # struct_time values in asctime() and strftime().
    # Re-enable this regression on WASI once the non-Unix time conversion path is fixed.

    # Regression test for RustPython issue #4938:
    # struct_time field overflow should raise OverflowError (matching CPython),
    # not TypeError. Covers mktime, asctime, and strftime.
    I32_MAX_PLUS_1 = 2147483648
    overflow_cases = [
        (I32_MAX_PLUS_1, 1, 1, 0, 0, 0, 0, 0, 0),  # i32 overflow in year
        (2024, I32_MAX_PLUS_1, 1, 0, 0, 0, 0, 0, 0),  # i32 overflow in month
        (2024, 1, I32_MAX_PLUS_1, 0, 0, 0, 0, 0, 0),  # i32 overflow in mday
        (2024, 1, 1, 0, 0, I32_MAX_PLUS_1, 0, 0, 0),  # i32 overflow in sec
        (88888888888,) * 9,  # multi-field i32 overflow
    ]

    for case in overflow_cases:
        for func_name, call in [
            ("mktime", lambda c=case: time.mktime(c)),
            ("asctime", lambda c=case: time.asctime(c)),
            ("strftime", lambda c=case: time.strftime("%Y", c)),
        ]:
            try:
                call()
            except OverflowError:
                pass  # expected, matches CPython
            except TypeError as e:
                raise AssertionError(
                    f"{func_name}({case}) raised TypeError (should be OverflowError): {e}"
                ) from e
            else:
                raise AssertionError(
                    f"{func_name}({case}) did not raise — expected OverflowError"
                )

s = time.asctime(x)
assert s == "Thu Jan  1 00:16:40 1970"

# Monotonic and performance clocks should advance with elapsed time.
monotonic_before = time.monotonic()
monotonic_ns = time.monotonic_ns()
monotonic_after = time.monotonic()

assert isinstance(monotonic_before, float)
assert isinstance(monotonic_ns, int)
assert monotonic_before <= monotonic_ns / 1_000_000_000 <= monotonic_after

perf_before = time.perf_counter()
perf_ns = time.perf_counter_ns()
perf_after = time.perf_counter()

assert isinstance(perf_before, float)
assert isinstance(perf_ns, int)
assert perf_before <= perf_ns / 1_000_000_000 <= perf_after

monotonic_start = time.monotonic()
perf_start = time.perf_counter()

time.sleep(0.02)

monotonic_elapsed = time.monotonic() - monotonic_start
perf_elapsed = time.perf_counter() - perf_start

assert monotonic_elapsed >= 0.01
assert perf_elapsed >= 0.01
