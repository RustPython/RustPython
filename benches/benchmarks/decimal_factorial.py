# Adapted from python/pyperformance 1.14.0 (decimal_factorial).
"""
Calculate `factorial` using the decimal module.

- 2024-06-14: Michael Droettboom copied this from
  Modules/_decimal/tests/bench.py in the CPython source and adapted to use
  pyperf.
"""

# Original copyright notice in CPython source:

#
# Copyright (C) 2001-2012 Python Software Foundation. All Rights Reserved.
# Modified and extended by Stefan Krah.
#


import decimal


def factorial(n, m):
    if n > m:
        return factorial(m, n)
    elif m == 0:
        return 1
    elif n == m:
        return n
    else:
        return factorial(n, (n + m) // 2) * factorial((n + m) // 2 + 1, m)


def bench_decimal_factorial():
    # The upstream 10,000! and 100,000! inputs are reduced for simulation,
    # while retaining enough precision to exercise large Decimal arithmetic.
    with decimal.localcontext() as context:
        context.prec = 3000
        result = factorial(decimal.Decimal(1000), 0)

    assert len(result.as_tuple().digits) == 2568


bench_decimal_factorial()
