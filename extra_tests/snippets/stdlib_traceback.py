import itertools
import traceback

import _suggestions
from testutils import assert_raises

try:
    1 / 0
except ZeroDivisionError as ex:
    tb = traceback.extract_tb(ex.__traceback__)
    assert len(tb) == 1


try:
    try:
        1 / 0
    except ZeroDivisionError as ex:
        raise KeyError().with_traceback(ex.__traceback__)
except KeyError as ex2:
    tb = traceback.extract_tb(ex2.__traceback__)
    assert tb[1].line == "1 / 0"


try:
    try:
        1 / 0
    except ZeroDivisionError as ex:
        raise ex.with_traceback(None)
except ZeroDivisionError as ex2:
    tb = traceback.extract_tb(ex2.__traceback__)
    assert len(tb) == 1

# The candidate list backing "Did you mean" suggestions is a list; an arbitrary
# iterable must be rejected rather than drained.

with assert_raises(TypeError):
    _suggestions._generate_suggestions(itertools.count(), "x")
assert _suggestions._generate_suggestions(["value"], "valu") == "value"
