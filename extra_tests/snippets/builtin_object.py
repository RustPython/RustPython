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

big = 1223456789812391231291231231231212312312312312312312321321321321312312321123123123199129391239219394923912949213021949302194942130123949203912430392402139210492139123012940219394923942395943856228368385
assert object.__sizeof__(big) >= big.__sizeof__()
