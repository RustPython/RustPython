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
assert f'{255:#X}' == "0XFF"
assert f"{65:c}" == "A"
assert f"{0x1f5a5:c}" == "🖥"
assert_raises(ValueError, "{:+c}".format, 1, _msg="Sign not allowed with integer format specifier 'c'")
assert_raises(ValueError, "{:#c}".format, 1, _msg="Alternate form (#) not allowed with integer format specifier 'c'")
assert f"{256:#010x}" == "0x00000100"
assert f"{256:0=#10x}" == "0x00000100"
assert f"{256:0>#10x}" == "000000x100"
assert f"{256:0^#10x}" == "000x100000"
assert f"{256:0<#10x}" == "0x10000000"
assert f"{512:+#010x}" == "+0x0000200"
assert f"{512:0=+#10x}" == "+0x0000200"
assert f"{512:0>+#10x}" == "0000+0x200"
assert f"{512:0^+#10x}" == "00+0x20000"
assert f"{512:0<+#10x}" == "+0x2000000"
