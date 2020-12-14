from testutils import assert_raises

def test_container(x):
    assert 3 in x
    assert 4 not in x
    assert list(x) == list(iter(x))
    assert list(x) == [0, 1, 2, 3]
    assert [*x] == [0, 1, 2, 3]
    lst = []
    lst.extend(x)
    assert lst == [0, 1, 2, 3]

class C:
    def __iter__(self):
        return iter([0, 1, 2, 3])
test_container(C())

class C:
    def __getitem__(self, x):
        return (0, 1, 2, 3)[x] # raises IndexError on x==4
test_container(C())

class C:
    def __getitem__(self, x):
        if x > 3:
            raise StopIteration
        return x
test_container(C())

class C: pass
assert_raises(TypeError, lambda: 5 in C())
assert_raises(TypeError, iter, C)

it = iter([1,2,3,4,5])
call_it = iter(lambda: next(it), 4)
assert list(call_it) == [1,2,3]
