assert 5 - 3 == 2

class Complex():
    def __init__(self, real, imag):
        self.real = real
        self.imag = imag

    def __repr__(self):
        return "Com" + str((self.real, self.imag))

    def __sub__(self, other):
        return Complex(self.real - other, self.imag)

    def __rsub__(self, other):
        return Complex(other - self.real, -self.imag)

    def __eq__(self, other):
        return self.real == other.real and self.imag == other.imag

assert Complex(4, 5) - 3 == Complex(1, 5)
assert 7 - Complex(4, 5) == Complex(3, -5)
