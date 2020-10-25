import math

from testutils import assert_raises

NAN = float('nan')
INF = float('inf')
NINF = float('-inf')

1 + 1.1

a = 1.2
b = 1.3
c = 1.2
z = 2
ov = 10 ** 1000

assert -a == -1.2

assert a < b
assert not b < a
assert a <= b
assert a <= c
assert a < z

assert b > a
assert not a > b
assert not a > c
assert b >= a
assert c >= a
assert not a >= b
assert z > a

assert a + b == 2.5
assert a - c == 0
assert a / c == 1
assert a % c == 0
assert a + z == 3.2
assert z + a == 3.2
assert a - z == -0.8
assert z - a == 0.8
assert a / z == 0.6
assert 6 / a == 5.0
assert 2.0 % z == 0.0
assert z % 2.0 == 0.0
assert_raises(OverflowError, lambda: a + ov)
assert_raises(OverflowError, lambda: a - ov)
assert_raises(OverflowError, lambda: a * ov)
assert_raises(OverflowError, lambda: a / ov)
assert_raises(OverflowError, lambda: a // ov)
assert_raises(OverflowError, lambda: a % ov)
assert_raises(OverflowError, lambda: a ** ov)
assert_raises(OverflowError, lambda: ov + a)
assert_raises(OverflowError, lambda: ov - a)
assert_raises(OverflowError, lambda: ov * a)
assert_raises(OverflowError, lambda: ov / a)
assert_raises(OverflowError, lambda: ov // a)
assert_raises(OverflowError, lambda: ov % a)
# assert_raises(OverflowError, lambda: ov ** a)

assert a < 5
assert a <= 5
assert a < 5.5
assert a <= 5.5
try:
    assert a < 'a'
except TypeError:
    pass
try:
    assert a <= 'a'
except TypeError:
    pass
assert a > 1
assert a >= 1
try:
    assert a > 'a'
except TypeError:
    pass
try:
    assert a >= 'a'
except TypeError:
    pass

assert math.isnan(float('nan'))
assert math.isnan(float('NaN'))
assert math.isnan(float('+NaN'))
assert math.isnan(float('-NaN'))

assert math.isinf(float('inf'))
assert math.isinf(float('Inf'))
assert math.isinf(float('+Inf'))
assert math.isinf(float('-Inf'))

assert float() == 0

assert float('+Inf') > 0
assert float('-Inf') < 0

assert float('3.14') == 3.14
assert float('2.99e-23') == 2.99e-23

assert float(b'3.14') == 3.14
assert float(b'2.99e-23') == 2.99e-23

assert_raises(ValueError, float, 'foo')
assert_raises(OverflowError, float, 2**10000)

# check eq and hash for small numbers

assert 1.0 == 1
assert 1.0 == True
assert 0.0 == 0
assert 0.0 == False
assert hash(1.0) == hash(1)
assert hash(1.0) == hash(True)
assert hash(0.0) == hash(0)
assert hash(0.0) == hash(False)
assert hash(1.0) != hash(1.0000000001)

assert 5.0 in {3, 4, 5}
assert {-1: 2}[-1.0] == 2

# check that magic methods are implemented for ints and floats

assert 1.0.__add__(1.0) == 2.0
assert 1.0.__radd__(1.0) == 2.0
assert 2.0.__sub__(1.0) == 1.0
assert 2.0.__rmul__(1.0) == 2.0
assert 1.0.__truediv__(2.0) == 0.5
assert 1.0.__rtruediv__(2.0) == 2.0
assert 2.5.__divmod__(2.0) == (1.0, 0.5)
assert 2.0.__rdivmod__(2.5) == (1.0, 0.5)

assert 1.0.__add__(1) == 2.0
assert 1.0.__radd__(1) == 2.0
assert 2.0.__sub__(1) == 1.0
assert 2.0.__rmul__(1) == 2.0
assert 1.0.__truediv__(2) == 0.5
assert 1.0.__rtruediv__(2) == 2.0
assert 2.0.__mul__(1) == 2.0
assert 2.0.__rsub__(1) == -1.0
assert 2.0.__mod__(2) == 0.0
assert 2.0.__rmod__(2) == 0.0
assert_raises(ZeroDivisionError, lambda: 2.0 / 0)
assert_raises(ZeroDivisionError, lambda: 2.0 // 0)
assert_raises(ZeroDivisionError, lambda: 2.0 % 0)
assert_raises(ZeroDivisionError, divmod, 2.0, 0)
assert_raises(ZeroDivisionError, lambda: 2 / 0.0)
assert_raises(ZeroDivisionError, lambda: 2 // 0.0)
assert_raises(ZeroDivisionError, lambda: 2 % 0.0)
# assert_raises(ZeroDivisionError, divmod, 2, 0.0)
assert_raises(ZeroDivisionError, lambda: 2.0 / 0.0)
assert_raises(ZeroDivisionError, lambda: 2.0 // 0.0)
assert_raises(ZeroDivisionError, lambda: 2.0 % 0.0)
assert_raises(ZeroDivisionError, divmod, 2.0, 0.0)

assert 1.2.__int__() == 1
assert 1.2.__float__() == 1.2
assert 1.2.__trunc__() == 1
assert int(1.2) == 1
assert float(1.2) == 1.2
assert math.trunc(1.2) == 1
assert_raises(OverflowError, float('inf').__trunc__)
assert_raises(ValueError, float('nan').__trunc__)
assert isinstance(0.5.__round__(), int)
assert isinstance(1.5.__round__(), int)
assert 0.5.__round__() == 0
assert 1.5.__round__() == 2
assert isinstance(0.5.__round__(0), float)
assert isinstance(1.5.__round__(0), float)
assert 0.5.__round__(0) == 0.0
assert 1.5.__round__(0) == 2.0
assert isinstance(0.5.__round__(None), int)
assert isinstance(1.5.__round__(None), int)
assert 0.5.__round__(None) == 0
assert 1.5.__round__(None) == 2
assert 1.234.__round__(1) == 1.2
assert 1.23456.__round__(4) == 1.2346
assert 1.00000000001.__round__(10) == 1.0
assert 1234.5.__round__(-2) == 1200
assert 1.234.__round__(-1) == 0
assert 1.23456789.__round__(15) == 1.23456789
assert 1.2e300.__round__(-500) == 0
assert 1.234.__round__(500) == 1.234
assert 1.2e-300.__round__(299) == 0
assert_raises(TypeError, lambda: 0.5.__round__(0.0))
assert_raises(TypeError, lambda: 1.5.__round__(0.0))
assert_raises(OverflowError, float('inf').__round__)
assert_raises(ValueError, float('nan').__round__)

assert 1.2 ** 2 == 1.44
assert_raises(OverflowError, lambda: 1.2 ** (10 ** 1000))
assert 3 ** 2.0 == 9.0

assert (1.7).real == 1.7
assert (1.7).imag == 0.0
assert (1.7).conjugate() == 1.7
assert (1.3).is_integer() == False
assert (1.0).is_integer()    == True

assert (0.875).as_integer_ratio() == (7, 8)
assert (-0.875).as_integer_ratio() == (-7, 8)
assert (0.0).as_integer_ratio() == (0, 1)
assert (11.5).as_integer_ratio() == (23, 2)
assert (0.0).as_integer_ratio() == (0, 1)
assert (2.5).as_integer_ratio() == (5, 2)
assert (0.5).as_integer_ratio() == (1, 2)
assert (2.1).as_integer_ratio() == (4728779608739021, 2251799813685248)
assert (-2.1).as_integer_ratio() == (-4728779608739021, 2251799813685248)
assert (-2100.0).as_integer_ratio() == (-2100, 1)
assert (2.220446049250313e-16).as_integer_ratio() == (1, 4503599627370496)
assert (1.7976931348623157e+308).as_integer_ratio() == (179769313486231570814527423731704356798070567525844996598917476803157260780028538760589558632766878171540458953514382464234321326889464182768467546703537516986049910576551282076245490090389328944075868508455133942304583236903222948165808559332123348274797826204144723168738177180919299881250404026184124858368, 1)
assert (2.2250738585072014e-308).as_integer_ratio() == (1, 44942328371557897693232629769725618340449424473557664318357520289433168951375240783177119330601884005280028469967848339414697442203604155623211857659868531094441973356216371319075554900311523529863270738021251442209537670585615720368478277635206809290837627671146574559986811484619929076208839082406056034304)

assert_raises(OverflowError, float('inf').as_integer_ratio)
assert_raises(OverflowError, float('-inf').as_integer_ratio)
assert_raises(ValueError, float('nan').as_integer_ratio)

assert str(1.0) == '1.0'
assert str(0.0) == '0.0'
assert str(-0.0) == '-0.0'
assert str(1.123456789) == '1.123456789'

# Test special case for lexer, float starts with a dot:
a = .5
assert a == 0.5
assert 3.14 == float('3.14')
src = """
a = 3._14
"""

with assert_raises(SyntaxError):
    exec(src)
src = """
a = 3.__14
"""

with assert_raises(SyntaxError):
    exec(src)

src = """
a = 3.1__4
"""

with assert_raises(SyntaxError):
    exec(src)

fromHex = float.fromhex
def identical(x, y):
    if math.isnan(x) or math.isnan(y):
        if math.isnan(x) == math.isnan(y):
            return
    elif x == y and (x != 0.0 or math.copysign(1.0, x) == math.copysign(1.0, y)):
        return
    raise SyntaxError(f"{x} not identical to {y}")

invalid_inputs = [
    "infi",  # misspelt infinities and nans
    "-Infinit",
    "++inf",
    "-+Inf",
    "--nan",
    "+-NaN",
    "snan",
    "NaNs",
    "nna",
    "an",
    "nf",
    "nfinity",
    "inity",
    "iinity",
    "0xnan",
    "",
    " ",
    "x1.0p0",
    "0xX1.0p0",
    "+ 0x1.0p0",  # internal whitespace
    "- 0x1.0p0",
    "0 x1.0p0",
    "0x 1.0p0",
    "0x1 2.0p0",
    "+0x1 .0p0",
    "0x1. 0p0",
    "-0x1.0 1p0",
    "-0x1.0 p0",
    "+0x1.0p +0",
    "0x1.0p -0",
    "0x1.0p 0",
    "+0x1.0p+ 0",
    "-0x1.0p- 0",
    "++0x1.0p-0",  # double signs
    "--0x1.0p0",
    "+-0x1.0p+0",
    "-+0x1.0p0",
    "0x1.0p++0",
    "+0x1.0p+-0",
    "-0x1.0p-+0",
    "0x1.0p--0",
    "0x1.0.p0",
    "0x.p0",  # no hex digits before or after point
    "0x1,p0",  # wrong decimal point character
    "0x1pa",
    "0x1p\uff10",  # fullwidth Unicode digits
    "\uff10x1p0",
    "0x\uff11p0",
    "0x1.\uff10p0",
    "0x1p0 \n 0x2p0",
    "0x1p0\0 0x1p0",  # embedded null byte is not end of string
]

for x in invalid_inputs:
    assert_raises(ValueError, lambda: fromHex(x))

value_pairs = [
    ("inf", INF),
    ("-Infinity", -INF),
    ("NaN", NAN),
    ("1.0", 1.0),
    ("-0x.2", -0.125),
    ("-0.0", -0.0),
]
whitespace = [
    "",
    " ",
    "\t",
    "\n",
    "\n \t",
    "\f",
    "\v",
    "\r"
]

for inp, expected in value_pairs:
    for lead in whitespace:
        for trail in whitespace:
            got = fromHex(lead + inp + trail)
            identical(got, expected)

MAX = fromHex('0x.fffffffffffff8p+1024')  # max normal
MIN = fromHex('0x1p-1022')                # min normal
TINY = fromHex('0x0.0000000000001p-1022') # min subnormal
EPS = fromHex('0x0.0000000000001p0') # diff between 1.0 and next float up

# two spellings of infinity, with optional signs; case-insensitive
identical(fromHex('inf'), INF)
identical(fromHex('+Inf'), INF)
identical(fromHex('-INF'), -INF)
identical(fromHex('iNf'), INF)
identical(fromHex('Infinity'), INF)
identical(fromHex('+INFINITY'), INF)
identical(fromHex('-infinity'), -INF)
identical(fromHex('-iNFiNitY'), -INF)

# nans with optional sign; case insensitive
identical(fromHex('nan'), NAN)
identical(fromHex('+NaN'), NAN)
identical(fromHex('-NaN'), NAN)
identical(fromHex('-nAN'), NAN)

# variations in input format
identical(fromHex('1'), 1.0)
identical(fromHex('+1'), 1.0)
identical(fromHex('1.'), 1.0)
identical(fromHex('1.0'), 1.0)
identical(fromHex('1.0p0'), 1.0)
identical(fromHex('01'), 1.0)
identical(fromHex('01.'), 1.0)
identical(fromHex('0x1'), 1.0)
identical(fromHex('0x1.'), 1.0)
identical(fromHex('0x1.0'), 1.0)
identical(fromHex('+0x1.0'), 1.0)
identical(fromHex('0x1p0'), 1.0)
identical(fromHex('0X1p0'), 1.0)
identical(fromHex('0X1P0'), 1.0)
identical(fromHex('0x1P0'), 1.0)
identical(fromHex('0x1.p0'), 1.0)
identical(fromHex('0x1.0p0'), 1.0)
identical(fromHex('0x.1p4'), 1.0)
identical(fromHex('0x.1p04'), 1.0)
identical(fromHex('0x.1p004'), 1.0)
identical(fromHex('0x1p+0'), 1.0)
identical(fromHex('0x1P-0'), 1.0)
identical(fromHex('+0x1p0'), 1.0)
identical(fromHex('0x01p0'), 1.0)
identical(fromHex('0x1p00'), 1.0)
identical(fromHex(' 0x1p0 '), 1.0)
identical(fromHex('\n 0x1p0'), 1.0)
identical(fromHex('0x1p0 \t'), 1.0)
identical(fromHex('0xap0'), 10.0)
identical(fromHex('0xAp0'), 10.0)
identical(fromHex('0xaP0'), 10.0)
identical(fromHex('0xAP0'), 10.0)
identical(fromHex('0xbep0'), 190.0)
identical(fromHex('0xBep0'), 190.0)
identical(fromHex('0xbEp0'), 190.0)
identical(fromHex('0XBE0P-4'), 190.0)
identical(fromHex('0xBEp0'), 190.0)
identical(fromHex('0xB.Ep4'), 190.0)
identical(fromHex('0x.BEp8'), 190.0)
identical(fromHex('0x.0BEp12'), 190.0)

# moving the point around
pi = fromHex('0x1.921fb54442d18p1')
identical(fromHex('0x.006487ed5110b46p11'), pi)
identical(fromHex('0x.00c90fdaa22168cp10'), pi)
identical(fromHex('0x.01921fb54442d18p9'), pi)
identical(fromHex('0x.03243f6a8885a3p8'), pi)
identical(fromHex('0x.06487ed5110b46p7'), pi)
identical(fromHex('0x.0c90fdaa22168cp6'), pi)
identical(fromHex('0x.1921fb54442d18p5'), pi)
identical(fromHex('0x.3243f6a8885a3p4'), pi)
identical(fromHex('0x.6487ed5110b46p3'), pi)
identical(fromHex('0x.c90fdaa22168cp2'), pi)
identical(fromHex('0x1.921fb54442d18p1'), pi)
identical(fromHex('0x3.243f6a8885a3p0'), pi)
identical(fromHex('0x6.487ed5110b46p-1'), pi)
identical(fromHex('0xc.90fdaa22168cp-2'), pi)
identical(fromHex('0x19.21fb54442d18p-3'), pi)
identical(fromHex('0x32.43f6a8885a3p-4'), pi)
identical(fromHex('0x64.87ed5110b46p-5'), pi)
identical(fromHex('0xc9.0fdaa22168cp-6'), pi)
identical(fromHex('0x192.1fb54442d18p-7'), pi)
identical(fromHex('0x324.3f6a8885a3p-8'), pi)
identical(fromHex('0x648.7ed5110b46p-9'), pi)
identical(fromHex('0xc90.fdaa22168cp-10'), pi)
identical(fromHex('0x1921.fb54442d18p-11'), pi)
identical(fromHex('0x1921fb54442d1.8p-47'), pi)
identical(fromHex('0x3243f6a8885a3p-48'), pi)
identical(fromHex('0x6487ed5110b46p-49'), pi)
identical(fromHex('0xc90fdaa22168cp-50'), pi)
identical(fromHex('0x1921fb54442d18p-51'), pi)
identical(fromHex('0x3243f6a8885a30p-52'), pi)
identical(fromHex('0x6487ed5110b460p-53'), pi)
identical(fromHex('0xc90fdaa22168c0p-54'), pi)
identical(fromHex('0x1921fb54442d180p-55'), pi)

assert (0.0).hex() == '0x0.0p+0'
assert (-0.0).hex() == '-0x0.0p+0'
assert (1.0).hex() == '0x1.0000000000000p+0'
assert (-1.5).hex() == '-0x1.8000000000000p+0'
assert float('inf').hex() == 'inf'
assert float('-inf').hex() == '-inf'
assert float('nan').hex() == 'nan'

# Test float exponent:
assert 1 if 1else 0 == 1

a = 3.
assert a.__eq__(3) is True
assert a.__eq__(3.) is True
assert a.__eq__(3.00000) is True
assert a.__eq__(3.01) is False

pi = 3.14
assert pi.__eq__(3.14) is True
assert pi.__ne__(3.14) is False
assert pi.__eq__(3) is False
assert pi.__ne__(3) is True
assert pi.__eq__('pi') is NotImplemented
assert pi.__ne__('pi') is NotImplemented

assert pi.__eq__(float('inf')) is False
assert pi.__ne__(float('inf')) is True
assert float('inf').__eq__(pi) is False
assert float('inf').__ne__(pi) is True
assert float('inf').__eq__(float('inf')) is True
assert float('inf').__ne__(float('inf')) is False
assert float('inf').__eq__(float('nan')) is False
assert float('inf').__ne__(float('nan')) is True

assert pi.__eq__(float('nan')) is False
assert pi.__ne__(float('nan')) is True
assert float('nan').__eq__(pi) is False
assert float('nan').__ne__(pi) is True
assert float('nan').__eq__(float('nan')) is False
assert float('nan').__ne__(float('nan')) is True
assert float('nan').__eq__(float('inf')) is False
assert float('nan').__ne__(float('inf')) is True

assert float(1e15).__repr__() == "1000000000000000.0"
assert float(1e16).__repr__() == "1e+16"
assert float(1e308).__repr__() == "1e+308"
assert float(1e309).__repr__() == "inf"
assert float(1e-323).__repr__() == "1e-323"
assert float(1e-324).__repr__() == "0.0"
assert float(1e-5).__repr__() == "1e-05"
assert float(1e-4).__repr__() == "0.0001"
assert float(1.2345678901234567890).__repr__() == "1.2345678901234567"
assert float(1.2345678901234567890e308).__repr__() == "1.2345678901234567e+308"

assert format(1e15) == "1000000000000000.0"
assert format(1e16) == "1e+16"
assert format(1e308) == "1e+308"
assert format(1e309) == "inf"
assert format(1e-323) == "1e-323"
assert format(1e-324) == "0.0"
assert format(1e-5) == "1e-05"
assert format(1e-4) == "0.0001"
assert format(1.2345678901234567890) == "1.2345678901234567"
assert format(1.2345678901234567890e308) == "1.2345678901234567e+308"

assert float('0_0') == 0.0
assert float('.0') == 0.0
assert float('0.') == 0.0
assert float('-.0') == 0.0
assert float('+.0') == 0.0

assert_raises(ValueError, lambda: float('0._0'))
assert_raises(ValueError, lambda: float('0_.0'))
assert_raises(ValueError, lambda: float('._0'))
assert_raises(ValueError, lambda: float('0_'))
assert_raises(ValueError, lambda: float('0._'))
assert_raises(ValueError, lambda: float('_.0'))
assert_raises(ValueError, lambda: float('._0'))
