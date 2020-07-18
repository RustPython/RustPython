from testutils import assert_raises

assert format(5, "b") == "101"

assert_raises(TypeError, format, 2, 3, _msg='format called with number')

assert format({}) == "{}"

assert_raises(TypeError, format, {}, 'b', _msg='format_spec not empty for dict')

class BadFormat:
    def __format__(self, spec):
        return 42
assert_raises(TypeError, format, BadFormat())

def test_zero_padding():
    i = 1
    assert f'{i:04d}' == '0001'

test_zero_padding()

assert '{:,}'.format(100) == '100'
assert '{:,}'.format(1024) == '1,024'
assert '{:_}'.format(65536) == '65_536'
assert '{:_}'.format(4294967296) == '4_294_967_296'
assert f'{100:_}' == '100'
assert f'{1024:_}' == '1_024'
assert f'{65536:,}' == '65,536'
assert f'{4294967296:,}' == '4,294,967,296'
assert 'F' == "{0:{base}}".format(15, base="X")
