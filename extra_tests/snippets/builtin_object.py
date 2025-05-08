class MyObject:
    pass


assert not MyObject() == MyObject()
assert MyObject() != MyObject()
myobj = MyObject()
assert myobj == myobj
assert not myobj != myobj

object.__subclasshook__(1) == NotImplemented

assert MyObject().__eq__(MyObject()) == NotImplemented
assert MyObject().__ne__(MyObject()) == NotImplemented
assert MyObject().__lt__(MyObject()) == NotImplemented
assert MyObject().__le__(MyObject()) == NotImplemented
assert MyObject().__gt__(MyObject()) == NotImplemented
assert MyObject().__ge__(MyObject()) == NotImplemented

obj = MyObject()

assert obj.__eq__(obj) is True
assert obj.__ne__(obj) is False

assert not hasattr(obj, "a")
obj.__dict__ = {"a": 1}
assert obj.a == 1

# Value inside the formatter goes through a different path of resolution.
# Check that it still works all the same
d = {
    0: "ab",
}
assert "ab ab" == "{k[0]} {vv}".format(k=d, vv=d[0])
