from testutils import TestFailingBool, TestFailingIter, assert_raises

assert all([True])
assert not all([False])
assert all([])
assert not all([False, TestFailingBool()])

assert_raises(RuntimeError, all, TestFailingIter())
assert_raises(RuntimeError, all, [TestFailingBool()])
