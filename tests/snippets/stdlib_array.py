from array import array

a1 = array("b", [0, 1, 2, 3])

assert a1.tobytes() == b"\x00\x01\x02\x03"
assert a1[2] == 2

assert list(a1) == [0, 1, 2, 3]

a1.reverse()
assert a1 == array("B", [3, 2, 1, 0])

a1.extend([4, 5, 6, 7])

assert a1 == array("h", [3, 2, 1, 0, 4, 5, 6, 7])
