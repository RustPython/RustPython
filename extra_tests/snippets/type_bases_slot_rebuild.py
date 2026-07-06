from testutils import assert_raises


# Reassigning __bases__ must rebuild slot dispatchers for the type and all its
# descendants: a slot whose method left the new MRO must be reset, not left stale.

# --- zelf itself loses __add__ (nb_add) ---
class OldAdd:
    def __add__(self, other):
        return "OLD"


class Bare:
    pass


class C(OldAdd):
    pass


c = C()
assert c + 1 == "OLD"
C.__bases__ = (Bare,)
with assert_raises(TypeError):
    c + 1


# --- 3-level descendant loses __iter__ (tp_iter) ---
class Itr:
    def __iter__(self):
        return iter([1, 2, 3])


class New:
    pass


class C2(Itr):
    pass


class D2(C2):
    pass


class E2(D2):
    pass


e = E2()
assert list(e) == [1, 2, 3]
C2.__bases__ = (New,)
with assert_raises(TypeError):
    list(e)


# --- descendant loses __len__ (sq_length), sibling slot untouched ---
class Sized:
    def __len__(self):
        return 7


class C3(Sized):
    pass


class D3(C3):
    pass


d3 = D3()
assert len(d3) == 7
C3.__bases__ = (Bare,)
with assert_raises(TypeError):
    len(d3)


# --- descendant loses __getitem__ (mp_subscript) ---
class Subscriptable:
    def __getitem__(self, key):
        return key * 2


class C4(Subscriptable):
    pass


class D4(C4):
    pass


d4 = D4()
assert d4[3] == 6
C4.__bases__ = (Bare,)
with assert_raises(TypeError):
    d4[3]


# --- descendant loses __call__ (tp_call) ---
class Callable:
    def __call__(self):
        return "called"


class C5(Callable):
    pass


class D5(C5):
    pass


d5 = D5()
assert d5() == "called"
C5.__bases__ = (Bare,)
with assert_raises(TypeError):
    d5()


# --- guard: stale-wrong-target, name present in both bases must switch ---
class OldTarget:
    def __add__(self, other):
        return "OLD.__add__"


class NewTarget:
    def __add__(self, other):
        return "NEW.__add__"


class C6(OldTarget):
    pass


class D6(C6):
    pass


d6 = D6()
assert d6 + 1 == "OLD.__add__"
C6.__bases__ = (NewTarget,)
assert d6 + 1 == "NEW.__add__"


# --- guard: __getattr__ resolves at call time, stays correct ---
class OldGetattr:
    def __getattr__(self, name):
        return "OLD:" + name


class C7(OldGetattr):
    pass


class D7(C7):
    pass


d7 = D7()
assert d7.missing == "OLD:missing"
C7.__bases__ = (Bare,)
with assert_raises(AttributeError):
    d7.missing


# --- mirror: new base ADDS a dunder the old chain lacked ---
class Adder:
    def __add__(self, other):
        return "ADDED"


class C8(Bare):
    pass


class D8(C8):
    pass


d8 = D8()
with assert_raises(TypeError):
    d8 + 1
C8.__bases__ = (Adder,)
assert d8 + 1 == "ADDED"


# --- round trip: swap away then back restores the slot ---
class C9(OldAdd):
    pass


class D9(C9):
    pass


d9 = D9()
assert d9 + 1 == "OLD"
C9.__bases__ = (Bare,)
with assert_raises(TypeError):
    d9 + 1
C9.__bases__ = (OldAdd,)
assert d9 + 1 == "OLD"
