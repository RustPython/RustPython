def foo():
    t = (1, 2, 3)
    t = (4, 5, 6)
    e = (
        (1, 2),
        (3, 4),
        1
    )
    return 3

foo.__jit__()
print(foo())