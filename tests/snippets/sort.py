class Stable:
    def __init__(self, id):
        self.id = id
    def __eq__(self, other):
        return True
    def __lt__(self, other):
        return False

l = [Stable(i) for i in range(10)]
l.sort()
assert [x.id for x in l] == list(range(10))
