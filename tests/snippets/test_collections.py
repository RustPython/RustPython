from collections import deque


d = deque([0, 1, 2])

d.append(1)
d.appendleft(3)

assert d == deque([3, 0, 1, 2, 1])

assert d <= deque([4])

assert d.copy() is not d

d = deque([1, 2, 3], 5)

d.extend([4, 5, 6])

assert d == deque([2, 3, 4, 5, 6])

d.remove(4)

assert d == deque([2, 3, 5, 6])

d.clear()

assert d == deque()

assert d == deque([], 4)
