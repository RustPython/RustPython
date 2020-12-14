from testutils import assert_raises
from testutils import TestFailingBool, TestFailingIter

assert any([True])
assert not any([False])
assert not any([])
assert any([True, TestFailingBool()])

assert_raises(RuntimeError, lambda: any(TestFailingIter()))
assert_raises(RuntimeError, lambda: any([TestFailingBool()]))
