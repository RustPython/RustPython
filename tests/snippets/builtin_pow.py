from testutils import assert_raises, assert_equal

assert pow(3, 2) == 9
assert pow(5, 3, 100) == 25

assert pow(41, 7, 2) == 1
assert pow(7, 2, 49) == 0


assert_almost_equal = assert_equal


def powtest(type):
    if type != float:
        for i in range(-1000, 1000):
            assert_equal(pow(type(i), 0), 1)
            assert_equal(pow(type(i), 1), type(i))
            assert_equal(pow(type(0), 1), type(0))
            assert_equal(pow(type(1), 1), type(1))

        for i in range(-100, 100):
            assert_equal(pow(type(i), 3), i * i * i)

        pow2 = 1
        for i in range(0, 31):
            assert_equal(pow(2, i), pow2)
            if i != 30:
                pow2 = pow2 * 2

        for othertype in (int,):
            for i in list(range(-10, 0)) + list(range(1, 10)):
                ii = type(i)
                for j in range(1, 11):
                    jj = -othertype(j)
                    pow(ii, jj)

    for othertype in int, float:
        for i in range(1, 100):
            zero = type(0)
            exp = -othertype(i / 10.0)
            if exp == 0:
                continue
            assert_raises(ZeroDivisionError, pow, zero, exp)

    il, ih = -20, 20
    jl, jh = -5,   5
    kl, kh = -10, 10
    asseq = assert_equal
    if type == float:
        il = 1
        asseq = assert_almost_equal
    elif type == int:
        jl = 0
    elif type == int:
        jl, jh = 0, 15
    for i in range(il, ih + 1):
        for j in range(jl, jh + 1):
            for k in range(kl, kh + 1):
                if k != 0:
                    if type == float or j < 0:
                        assert_raises(TypeError, pow, type(i), j, k)
                        continue
                    asseq(
                        pow(type(i), j, k),
                        pow(type(i), j) % type(k)
                    )


def test_powint():
    powtest(int)


def test_powfloat():
    powtest(float)


def test_other():
    # Other tests-- not very systematic
    assert_equal(pow(3,3) % 8, pow(3,3,8))
    assert_equal(pow(3,3) % -8, pow(3,3,-8))
    assert_equal(pow(3,2) % -2, pow(3,2,-2))
    assert_equal(pow(-3,3) % 8, pow(-3,3,8))
    assert_equal(pow(-3,3) % -8, pow(-3,3,-8))
    assert_equal(pow(5,2) % -8, pow(5,2,-8))

    assert_equal(pow(3,3) % 8, pow(3,3,8))
    assert_equal(pow(3,3) % -8, pow(3,3,-8))
    assert_equal(pow(3,2) % -2, pow(3,2,-2))
    assert_equal(pow(-3,3) % 8, pow(-3,3,8))
    assert_equal(pow(-3,3) % -8, pow(-3,3,-8))
    assert_equal(pow(5,2) % -8, pow(5,2,-8))

    for i in range(-10, 11):
        for j in range(0, 6):
            for k in range(-7, 11):
                if j >= 0 and k != 0:
                    assert_equal(
                        pow(i,j) % k,
                        pow(i,j,k)
                    )
                if j >= 0 and k != 0:
                    assert_equal(
                        pow(int(i),j) % k,
                        pow(int(i),j,k)
                    )


def test_bug643260():
    class TestRpow:
        def __rpow__(self, other):
            return None
    None ** TestRpow() # Won't fail when __rpow__ invoked.  SF bug #643260.


def test_bug705231():
    # -1.0 raised to an integer should never blow up.  It did if the
    # platform pow() was buggy, and Python didn't worm around it.
    eq = assert_equal
    a = -1.0
    # The next two tests can still fail if the platform floor()
    # function doesn't treat all large inputs as integers
    # test_math should also fail if that is happening
    eq(pow(a, 1.23e167), 1.0)
    eq(pow(a, -1.23e167), 1.0)
    for b in range(-10, 11):
        eq(pow(a, float(b)), b & 1 and -1.0 or 1.0)
    for n in range(0, 100):
        fiveto = float(5 ** n)
        # For small n, fiveto will be odd.  Eventually we run out of
        # mantissa bits, though, and thereafer fiveto will be even.
        expected = fiveto % 2.0 and -1.0 or 1.0
        eq(pow(a, fiveto), expected)
        eq(pow(a, -fiveto), expected)
    eq(expected, 1.0)   # else we didn't push fiveto to evenness


tests = [f for name, f in locals().items() if name.startswith('test_')]
for f in tests:
    f()
