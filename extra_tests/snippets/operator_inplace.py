class InPlace:
    def __init__(self, val):
        self.val = val

    def __ipow__(self, other):
        self.val **= other
        return self

    def __imul__(self, other):
        self.val *= other
        return self

    def __imatmul__(self, other):
        # I guess you could think of an int as a 1x1 matrix
        self.val *= other
        return self

    def __itruediv__(self, other):
        self.val /= other
        return self

    def __ifloordiv__(self, other):
        self.val //= other
        return self

    def __imod__(self, other):
        self.val %= other
        return self

    def __iadd__(self, other):
        self.val += other
        return self

    def __isub__(self, other):
        self.val -= other
        return self

    def __ilshift__(self, other):
        self.val <<= other
        return self

    def __irshift__(self, other):
        self.val >>= other
        return self

    def __iand__(self, other):
        self.val &= other
        return self

    def __ixor__(self, other):
        self.val ^= other
        return self

    def __ior__(self, other):
        self.val |= other
        return self


i = InPlace(2)
i **= 3
assert i.val == 8

i = InPlace(2)
i *= 2
assert i.val == 4

i = InPlace(2)
i @= 2
assert i.val == 4

i = InPlace(1)
i /= 2
assert i.val == 0.5

i = InPlace(1)
i //= 2
assert i.val == 0

i = InPlace(10)
i %= 3
assert i.val == 1

i = InPlace(1)
i += 1
assert i.val == 2

i = InPlace(2)
i -= 1
assert i.val == 1

i = InPlace(2)
i <<= 3
assert i.val == 16

i = InPlace(16)
i >>= 3
assert i.val == 2

i = InPlace(0b010101)
i &= 0b111000
assert i.val == 0b010000

i = InPlace(0b010101)
i ^= 0b111000
assert i.val == 0b101101

i = InPlace(0b010101)
i |= 0b111000
assert i.val == 0b111101
