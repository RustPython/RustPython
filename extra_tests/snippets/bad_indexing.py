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


def run_setslice():
    with assert_raises(IndexError):
        e[BadIndex()] = 42
    e[BadIndex():0:-1] = e
    e[0:BadIndex():1] = e
    e[0:10:BadIndex()] = e


def run_delslice():
    del e[BadIndex():0:-1]
    del e[0:BadIndex():1]
    del e[0:10:BadIndex()]

# Check types 
instances = [list(), bytearray(), array('b')]
for e in instances:
    run_setslice()
    run_delslice()