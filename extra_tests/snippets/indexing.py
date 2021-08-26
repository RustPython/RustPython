""" Test that indexing ops don't hang when an object with a mutating
__index__ is used."""
from testutils import assert_raises
from array import array


class BadIndex:
    def __index__(self):
        # assign ourselves, makes it easy to re-use with
        # all mutable collections.
        e[:] = e
        return 1


def run_setitem():
    with assert_raises(IndexError):
        e[BadIndex()] = 42
    e[BadIndex():0:-1] = e
    e[0:BadIndex():1] = e
    e[0:10:BadIndex()] = e


def run_delitem():
    del e[BadIndex():0:-1]
    del e[0:BadIndex():1]
    del e[0:10:BadIndex()]

# Check list
e = []
run_setitem()
run_delitem()

# Check bytearray
e = bytearray()
run_setitem()
run_delitem()

# Check array
e = array('b')
run_setitem()
run_delitem()