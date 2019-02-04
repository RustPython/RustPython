x = [val for val in range(5)]

assert x == [0,1,2,3,4], "range __iter__ failed"

assert len(x) == 5, "range __len__ failed"

y = range(0, -5, -1)

assert list(y) == [0,-1,-2,-3,-4], "backward range failed"

assert y[2] == -2, "range indexing failed"

assert list(y[1:3:2]) == [-1], "range slicing failed"
