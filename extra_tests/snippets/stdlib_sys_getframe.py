import sys

value = 189
locals_dict = sys._getframe().f_locals
assert locals_dict['value'] == 189
foo = 'bar'
assert locals_dict['foo'] == foo

def test_function():
    x = 17
    assert sys._getframe().f_locals is not locals_dict
    assert sys._getframe().f_locals['x'] == 17
    assert sys._getframe(1).f_locals['foo'] == 'bar'

test_function()

class TestClass():
    def __init__(self):
        assert sys._getframe().f_locals['self'] == self

TestClass()
