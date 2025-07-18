from testutils import assert_raises

assert format(5, "b") == "101"

assert_raises(TypeError, format, 2, 3, _msg="format called with number")

assert format({}) == "{}"

assert_raises(TypeError, format, {}, "b", _msg="format_spec not empty for dict")


class BadFormat:
    def __format__(self, spec):
        return 42


assert_raises(TypeError, format, BadFormat())


def test_zero_padding():
    i = 1
    assert f"{i:04d}" == "0001"


test_zero_padding()

assert "{:,}".format(100) == "100"
assert "{:,}".format(1024) == "1,024"
assert "{:_}".format(65536) == "65_536"
assert "{:_}".format(4294967296) == "4_294_967_296"
assert f"{100:_}" == "100"
assert f"{1024:_}" == "1_024"
assert f"{65536:,}" == "65,536"
assert f"{4294967296:,}" == "4,294,967,296"
assert "F" == "{0:{base}}".format(15, base="X")
assert f"{255:#X}" == "0XFF"
assert f"{65:c}" == "A"
assert f"{0x1F5A5:c}" == "🖥"
assert_raises(
    ValueError,
    "{:+c}".format,
    1,
    _msg="Sign not allowed with integer format specifier 'c'",
)
assert_raises(
    ValueError,
    "{:#c}".format,
    1,
    _msg="Alternate form (#) not allowed with integer format specifier 'c'",
)
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
assert f"{1234:10}" == "      1234"
assert f"{1234:10,}" == "     1,234"
assert f"{1234:010,}" == "00,001,234"
assert f"{'🐍':4}" == "🐍   "
assert_raises(
    ValueError, "{:,o}".format, 1, _msg="ValueError: Cannot specify ',' with 'o'."
)
assert_raises(
    ValueError, "{:_n}".format, 1, _msg="ValueError: Cannot specify '_' with 'n'."
)
assert_raises(
    ValueError, "{:,o}".format, 1.0, _msg="ValueError: Cannot specify ',' with 'o'."
)
assert_raises(
    ValueError, "{:_n}".format, 1.0, _msg="ValueError: Cannot specify '_' with 'n'."
)
assert_raises(
    ValueError, "{:,}".format, "abc", _msg="ValueError: Cannot specify ',' with 's'."
)
assert_raises(
    ValueError, "{:,x}".format, "abc", _msg="ValueError: Cannot specify ',' with 'x'."
)
assert_raises(
    OverflowError,
    "{:c}".format,
    0x110000,
    _msg="OverflowError: %c arg not in range(0x110000)",
)
assert f"{3:f}" == "3.000000"
assert f"{3.1415:.0f}" == "3"
assert f"{3.1415:.1f}" == "3.1"
assert f"{3.1415:.2f}" == "3.14"
assert f"{3.1415:.3f}" == "3.142"
assert f"{3.1415:.4f}" == "3.1415"
assert f"{3.1415:#.0f}" == "3."
assert f"{3.1415:#.1f}" == "3.1"
assert f"{3.1415:#.2f}" == "3.14"
assert f"{3.1415:#.3f}" == "3.142"
assert f"{3.1415:#.4f}" == "3.1415"
assert f"{3:g}" == "3"
assert f"{3.1415:.0g}" == "3"
assert f"{3.1415:.1g}" == "3"
assert f"{3.1415:.2g}" == "3.1"
assert f"{3.1415:.3g}" == "3.14"
assert f"{3.1415:.4g}" == "3.142"
assert f"{0.000012:g}" == "1.2e-05"
assert f"{0.000012:G}" == "1.2E-05"
assert f"{3:#g}" == "3.00000"
assert f"{3.1415:#.0g}" == "3."
assert f"{3.1415:#.1g}" == "3."
assert f"{3.1415:#.2g}" == "3.1"
assert f"{3.1415:#.3g}" == "3.14"
assert f"{3.1415:#.4g}" == "3.142"
assert f"{0.000012:#g}" == "1.20000e-05"
assert f"{0.000012:#G}" == "1.20000E-05"
assert f"{3.1415:.0e}" == "3e+00"
assert f"{3.1415:.1e}" == "3.1e+00"
assert f"{3.1415:.2e}" == "3.14e+00"
assert f"{3.1415:.3e}" == "3.142e+00"
assert f"{3.1415:.4e}" == "3.1415e+00"
assert f"{3.1415:.5e}" == "3.14150e+00"
assert f"{3.1415:.5E}" == "3.14150E+00"
assert f"{3.1415:#.0e}" == "3.e+00"
assert f"{3.1415:#.1e}" == "3.1e+00"
assert f"{3.1415:#.2e}" == "3.14e+00"
assert f"{3.1415:#.3e}" == "3.142e+00"
assert f"{3.1415:#.4e}" == "3.1415e+00"
assert f"{3.1415:#.5e}" == "3.14150e+00"
assert f"{3.1415:#.5E}" == "3.14150E+00"
assert f"{3.1415:.0%}" == "314%"
assert f"{3.1415:.1%}" == "314.2%"
assert f"{3.1415:.2%}" == "314.15%"
assert f"{3.1415:.3%}" == "314.150%"
assert f"{3.1415:#.0%}" == "314.%"
assert f"{3.1415:#.1%}" == "314.2%"
assert f"{3.1415:#.2%}" == "314.15%"
assert f"{3.1415:#.3%}" == "314.150%"
assert f"{3.1415:.0}" == "3e+00"
assert f"{3.1415:.1}" == "3e+00"
assert f"{3.1415:.2}" == "3.1"
assert f"{3.1415:.3}" == "3.14"
assert f"{3.1415:.4}" == "3.142"
assert f"{3.1415:#.0}" == "3.e+00"
assert f"{3.1415:#.1}" == "3.e+00"
assert f"{3.1415:#.2}" == "3.1"
assert f"{3.1415:#.3}" == "3.14"
assert f"{3.1415:#.4}" == "3.142"
assert f"{1234.5:10}" == "    1234.5"
assert f"{1234.5:10,}" == "   1,234.5"
assert f"{1234.5:010,}" == "0,001,234.5"
assert f"{12.34 + 5.6j}" == "(12.34+5.6j)"
assert f"{12.34 - 5.6j: }" == "( 12.34-5.6j)"
assert f"{12.34 + 5.6j:20}" == "        (12.34+5.6j)"
assert f"{12.34 + 5.6j:<20}" == "(12.34+5.6j)        "
assert f"{-12.34 + 5.6j:^20}" == "   (-12.34+5.6j)    "
assert f"{12.34 + 5.6j:^+20}" == "   (+12.34+5.6j)    "
assert f"{12.34 + 5.6j:_^+20}" == "___(+12.34+5.6j)____"
assert f"{-12.34 + 5.6j:f}" == "-12.340000+5.600000j"
assert f"{12.34 + 5.6j:.3f}" == "12.340+5.600j"
assert f"{12.34 + 5.6j:<30.8f}" == "12.34000000+5.60000000j       "
assert f"{12.34 + 5.6j:g}" == "12.34+5.6j"
assert f"{12.34 + 5.6j:e}" == "1.234000e+01+5.600000e+00j"
assert f"{12.34 + 5.6j:E}" == "1.234000E+01+5.600000E+00j"
assert f"{12.34 + 5.6j:^30E}" == "  1.234000E+01+5.600000E+00j  "
assert f"{12345.6 + 7890.1j:,}" == "(12,345.6+7,890.1j)"
assert f"{12345.6 + 7890.1j:_.3f}" == "12_345.600+7_890.100j"
assert f"{12345.6 + 7890.1j:>+30,f}" == "  +12,345.600000+7,890.100000j"

# test issue 4558
x = 123456789012345678901234567890
for i in range(0, 30):
    format(x, ",")
    x = x // 10
