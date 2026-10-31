"""Test indexing ops with a key whose __index__ misbehaves: they must not
hang when it mutates the sequence, and must propagate its exception."""

import operator
from array import array

from testutils import assert_raises


class BadIndex:
    def __index__(self):
        # assign ourselves, makes it easy to re-use with
        # all mutable collections.
        e[:] = e
        return 1


def run_setslice():
    with assert_raises(IndexError):
        e[BadIndex()] = 42
    e[BadIndex() : 0 : -1] = e
    e[0 : BadIndex() : 1] = e
    e[0 : 10 : BadIndex()] = e


def run_delslice():
    del e[BadIndex() : 0 : -1]
    del e[0 : BadIndex() : 1]
    del e[0 : 10 : BadIndex()]


# Check types
instances = [list(), bytearray(), array("b")]
for e in instances:
    run_setslice()
    run_delslice()


class RaisingIndex:
    def __index__(self):
        raise ZeroDivisionError


class Plain:
    pass


t = ()
with assert_raises(ZeroDivisionError):
    t[RaisingIndex()] = 0
with assert_raises(ZeroDivisionError):
    del t[RaisingIndex()]
with assert_raises(ZeroDivisionError):
    operator.setitem(b"", RaisingIndex(), 0)
with assert_raises(ZeroDivisionError):
    operator.delitem("", RaisingIndex())
with assert_raises(ZeroDivisionError):
    Plain()[RaisingIndex()] = 0
with assert_raises(ZeroDivisionError):
    del Plain()[RaisingIndex()]
with assert_raises(IndexError):
    t[2**70] = 0
with assert_raises(IndexError):
    del t[2**70]

with assert_raises(TypeError) as cm:
    del t[0]
assert str(cm.exception) == "'tuple' object doesn't support item deletion"
with assert_raises(TypeError) as cm:
    del t["a"]
assert str(cm.exception) == "'tuple' object does not support item deletion"
with assert_raises(TypeError) as cm:
    del object()[RaisingIndex()]
assert str(cm.exception) == "'object' object does not support item deletion"
