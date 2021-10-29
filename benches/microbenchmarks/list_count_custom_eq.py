class A:
    def __eq__(self, other):
        return True

l = [A()] * ITERATIONS

# ---
l.count(1)