class MyObject:
    pass

assert not MyObject() == MyObject()
assert MyObject() != MyObject()
myobj = MyObject()
assert myobj == myobj
assert not myobj != myobj

assert MyObject().__eq__(MyObject()) == NotImplemented
assert MyObject().__ne__(MyObject()) == NotImplemented
assert MyObject().__lt__(MyObject()) == NotImplemented
assert MyObject().__le__(MyObject()) == NotImplemented
assert MyObject().__gt__(MyObject()) == NotImplemented
assert MyObject().__ge__(MyObject()) == NotImplemented
