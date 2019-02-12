class MyObject:
    pass

assert not MyObject() == MyObject()
assert MyObject() != MyObject()
myobj = MyObject()
assert myobj == myobj
assert not myobj != myobj

assert MyObject().__eq__(MyObject()) == NotImplemented
assert MyObject().__ne__(MyObject()) == NotImplemented
