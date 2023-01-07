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
assert f"{123:,}" == "123"
assert f"{1234:,}" == "1,234"
assert f"{12345:,}" == "12,345"
assert f"{123456:,}" == "123,456"
assert f"{123:03_}" == "123"
assert f"{123:04_}" == "0_123"
assert f"{123:05_}" == "0_123"
assert f"{123:06_}" == "00_123"
assert f"{123:07_}" == "000_123"
assert f"{255:#010_x}" == "0x000_00ff"
assert f"{255:+#010_x}" == "+0x00_00ff"
assert f"{123.4567:,}" == "123.4567"
assert f"{1234.567:,}" == "1,234.567"
assert f"{12345.67:,}" == "12,345.67"
assert f"{123456.7:,}" == "123,456.7"
assert f"{123.456:07,}" == "123.456"
assert f"{123.456:08,}" == "0,123.456"
assert f"{123.456:09,}" == "0,123.456"
assert f"{123.456:010,}" == "00,123.456"
assert f"{123.456:011,}" == "000,123.456"
assert f"{123.456:+011,}" == "+00,123.456"
assert f"{1234:.3g}" == "1.23e+03"
assert f"{1234567:.6G}" == "1.23457E+06"
assert f'{"🐍":4}' == "🐍   "
assert_raises(ValueError, "{:,o}".format, 1, _msg="ValueError: Cannot specify ',' with 'o'.")
assert_raises(ValueError, "{:_n}".format, 1, _msg="ValueError: Cannot specify '_' with 'n'.")
assert_raises(ValueError, "{:,o}".format, 1.0, _msg="ValueError: Cannot specify ',' with 'o'.")
assert_raises(ValueError, "{:_n}".format, 1.0, _msg="ValueError: Cannot specify '_' with 'n'.")
assert_raises(ValueError, "{:,}".format, "abc", _msg="ValueError: Cannot specify ',' with 's'.")
assert_raises(ValueError, "{:,x}".format, "abc", _msg="ValueError: Cannot specify ',' with 'x'.")
assert_raises(OverflowError, "{:c}".format, 0x110000, _msg="OverflowError: %c arg not in range(0x110000)")
