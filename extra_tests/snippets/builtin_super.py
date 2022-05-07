test_super_list = super(list)
assert test_super_list.__self__ is None
assert test_super_list.__self_class__ is None
assert test_super_list.__thisclass__ == list


class testA:
    a = 1


class testB(testA):
    b = 1


superB = super(testB)
assert superB.__thisclass__ == testB
assert superB.__self_class__ is None
assert superB.__self__ is None
