import testutils

# __complex__
def test__complex__():
    z = 3 + 4j
    assert z.__complex__() == z
    assert type(z.__complex__()) == complex
    
    class complex_subclass(complex):
        pass
    z = complex_subclass(3 + 4j)
    assert z.__complex__() == 3 + 4j
    assert type(z.__complex__()) == complex

testutils.skip_if_unsupported(3,11,test__complex__)