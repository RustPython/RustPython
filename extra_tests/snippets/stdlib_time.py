import time

x = time.gmtime(1000)

assert x.tm_year == 1970
assert x.tm_min == 16
assert x.tm_sec == 40
assert x.tm_isdst == 0

s = time.strftime("%Y-%m-%d-%H-%M-%S", x)
# print(s)
assert s == "1970-01-01-00-16-40"

x2 = time.strptime(s, "%Y-%m-%d-%H-%M-%S")
assert x2.tm_min == 16

s = time.asctime(x)
# print(s)
assert s == "Thu Jan  1 00:16:40 1970"


# Regression test for RustPython issue #4938:
# struct_time field overflow should raise OverflowError (matching CPython),
# not TypeError. Covers mktime, asctime, and strftime.
I32_MAX_PLUS_1 = 2147483648
overflow_cases = [
    (I32_MAX_PLUS_1, 1, 1, 0, 0, 0, 0, 0, 0),       # i32 overflow in year
    (2024, I32_MAX_PLUS_1, 1, 0, 0, 0, 0, 0, 0),    # i32 overflow in month
    (2024, 1, I32_MAX_PLUS_1, 0, 0, 0, 0, 0, 0),    # i32 overflow in mday
    (2024, 1, 1, 0, 0, I32_MAX_PLUS_1, 0, 0, 0),    # i32 overflow in sec
    (88888888888,) * 9,                              # multi-field i32 overflow
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
            )
        else:
            raise AssertionError(
                f"{func_name}({case}) did not raise — expected OverflowError"
            )
