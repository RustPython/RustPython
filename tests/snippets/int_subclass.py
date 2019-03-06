import dis

class A(int):
    def __init__(self):
        self.x = "attr"

a = A()

assert 2 * a == 0
assert a.x == "attr"
