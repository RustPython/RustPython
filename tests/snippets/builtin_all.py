from testutils import assert_raises
from testutils import TestFailingBool, TestFailingIter

assert all([True])
assert not all([False])
assert all([])
assert not all([False, TestFailingBool()])

assert_raises(RuntimeError, lambda: all(TestFailingIter()))
assert_raises(RuntimeError, lambda: all([TestFailingBool()]))
