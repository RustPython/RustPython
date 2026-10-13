import sys
import time

from testutils import assert_raises

if sys.platform == "win32":
    # Record the native runtime before evaluating the range regression below.
    print("Windows time runtime:", sys.version, sys.implementation, sys.flags)
    print("Windows time word size:", sys.maxsize)
    if hasattr(sys, "getwindowsversion"):
        print("Windows version:", sys.getwindowsversion())
    try:
        import sysconfig

        build_keys = (
            "Py_DEBUG",
            "Py_GIL_DISABLED",
            "Py_ENABLE_SHARED",
            "SIZEOF_TIME_T",
            "CC",
            "CFLAGS",
        )
        print(
            "Windows time build:",
            dict(zip(build_keys, sysconfig.get_config_vars(*build_keys))),
        )
    except Exception as error:
        print("Windows time build unavailable:", type(error).__name__, str(error))
    for converter in (time.gmtime, time.localtime, time.ctime):
        for timestamp in (
            0,
            -1,
            -43200,
            -43201,
            -86400,
            2**40,
            2**63 - 1,
            -(2**63),
            -float(2**63),
            float(2**63),
        ):
            try:
                result = converter(timestamp)
            except Exception as error:
                print(
                    "Windows time result:",
                    converter.__name__,
                    repr(timestamp),
                    type(error).__name__,
                    str(error),
                    getattr(error, "errno", None),
                    getattr(error, "winerror", None),
                )
            else:
                print(
                    "Windows time result:",
                    converter.__name__,
                    repr(timestamp),
                    "success",
                    repr(result),
                )

if sys.platform != "wasi":
    for converter in (time.gmtime, time.localtime, time.ctime):
        # 2**63 is outside signed time_t even though its maximum rounds up to
        # that value when represented as a float.
        try:
            converter(float(2**63))
        except OverflowError:
            pass
        else:
            raise AssertionError("out-of-range timestamp did not raise OverflowError")

if sys.platform == "win32":
    import errno

    for converter in (time.gmtime, time.localtime, time.ctime):
        # Non-negative timestamps still use UCRT and preserve errno.
        for timestamp in (2**40, 2**63 - 1):
            try:
                converter(timestamp)
            except OSError as error:
                assert error.errno == errno.EINVAL, (converter, timestamp, error)
            else:
                raise AssertionError("UCRT timestamp failure did not raise OSError")

        # Negative timestamps use FILETIME, whose epoch is 1601-01-01.
        for timestamp in (-11_644_473_601, -(2**63), -float(2**63)):
            try:
                converter(timestamp)
            except OverflowError as error:
                assert str(error) == "timestamp out of range for Windows FILETIME"
            else:
                raise AssertionError(
                    "pre-FILETIME timestamp did not raise OverflowError"
                )

    assert time.gmtime(-11_644_473_600) == (1601, 1, 1, 0, 0, 0, 0, 1, 0)
    assert time.gmtime(-86400) == (1969, 12, 31, 0, 0, 0, 2, 365, 0)
    assert time.gmtime(-1) == (1969, 12, 31, 23, 59, 59, 2, 365, 0)
    # Leap-year accounting uses the historical date, including century rules.
    assert time.gmtime(-2_203_891_200).tm_yday == 60  # 1900-03-01
    assert time.gmtime(-2_330_035_200).tm_yday == 61  # 1896-03-01
    for timestamp in (-15_897_600, -86400, -43201, -1):
        local = time.localtime(timestamp)
        assert local.tm_isdst == -1
        assert local[:6] == time.gmtime(timestamp + local.tm_gmtoff)[:6]
        assert time.ctime(timestamp) == time.asctime(local)

x = time.gmtime(1000)

assert x.tm_year == 1970
assert x.tm_min == 16
assert x.tm_sec == 40
assert x.tm_isdst == 0

s = time.strftime("%Y-%m-%d-%H-%M-%S", x)
# print(s)
assert s == "1970-01-01-00-16-40"

if sys.platform in ("linux", "darwin"):
    import locale

    old_locale = locale.setlocale(locale.LC_ALL)
    try:
        # Locale output must be Unicode even when the locale is not UTF-8.
        # Keep LC_CTYPE and LC_TIME consistent for wcsftime on macOS.
        # These locales are optional on the host running the snippet.
        for time_locale in ("fr_FR.ISO8859-1", "fr_FR.UTF-8"):
            try:
                locale.setlocale(locale.LC_ALL, time_locale)
            except locale.Error:
                continue
            february = (2024, 2, 1, 12, 0, 0, 3, 32, 0)
            assert time.strftime("%B", february) == "février"
            assert (
                time.strftime("[%B] 🐍 \ud800\0%Y", february)
                == "[février] 🐍 \ud800\0" + "2024"
            )
            assert time.strftime("%B" * 300, february) == "février" * 300
    finally:
        locale.setlocale(locale.LC_ALL, old_locale)

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

# Sleep validates signed nanoseconds before rejecting negative durations.
for duration in (
    2**63,
    -(2**63) - 1,
    2**34,
    -(2**34),
    2**63 / 1_000_000_000,
    -1e10,
    float("inf"),
    -float("inf"),
):
    with assert_raises(OverflowError) as error:
        time.sleep(duration)
    assert str(error.exception) == "timestamp out of range for C PyTime_t"
assert_raises(ValueError, time.sleep, float("nan"))


class SleepTypeNamespace:
    class Invalid:
        pass


for module in ("__main__", "sleep_test"):
    SleepTypeNamespace.Invalid.__module__ = module
    with assert_raises(TypeError) as error:
        time.sleep(SleepTypeNamespace.Invalid())
    assert str(error.exception) == "must be real number, not Invalid"


class FloatSleepDuration:
    def __float__(self):
        return -0.5


class IndexedSleepDuration(FloatSleepDuration):
    def __index__(self):
        return -1

    def __float__(self):
        raise AssertionError("sleep must prefer __index__")


class InvalidIndexedSleepDuration(FloatSleepDuration):
    def __index__(self):
        raise TypeError("index conversion failed")


for duration in (FloatSleepDuration(), IndexedSleepDuration()):
    with assert_raises(ValueError) as error:
        time.sleep(duration)
    assert str(error.exception) == "sleep length must be non-negative"

with assert_raises(TypeError) as error:
    time.sleep(InvalidIndexedSleepDuration())
assert str(error.exception) == "index conversion failed"

# The optional second argument fills the fields that are not part of the
# sequence.
fields = (2024, 1, 2, 3, 4, 5, 6, 7, 0)
assert time.struct_time(fields).tm_zone is None
assert time.struct_time(fields, {"tm_zone": "UTC"}).tm_zone == "UTC"
assert time.struct_time(fields, {"tm_gmtoff": 60}).tm_gmtoff == 60
try:
    time.struct_time(fields, ["tm_zone", "UTC"])
except TypeError:
    pass
else:
    assert False, "struct_time accepted a non-dict second argument"
