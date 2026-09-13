import array

from testutils import assert_raises

obj = b"abcde"
a = memoryview(obj)
assert a.obj == obj

assert a[2:3] == b"c"

assert hash(obj) == hash(a)


class A(array.array): ...


class B(bytes): ...


class C: ...


memoryview(bytearray("abcde", encoding="utf-8"))
memoryview(array.array("i", [1, 2, 3]))
memoryview(A("b", [0]))
memoryview(B("abcde", encoding="utf-8"))

assert_raises(TypeError, lambda: memoryview([1, 2, 3]))
assert_raises(TypeError, lambda: memoryview((1, 2, 3)))
assert_raises(TypeError, lambda: memoryview({}))
assert_raises(TypeError, lambda: memoryview("string"))
assert_raises(TypeError, lambda: memoryview(C()))


def test_slice():
    b = b"123456789"
    m = memoryview(b)
    m2 = memoryview(b)
    assert m == m
    assert m == m2
    assert m.tobytes() == b"123456789"
    assert m == b
    assert m[::2].tobytes() == b"13579"
    assert m[::2] == b"13579"
    assert m[1::2].tobytes() == b"2468"
    assert m[::2][1:].tobytes() == b"3579"
    assert m[::2][1:-1].tobytes() == b"357"
    assert m[::2][::2].tobytes() == b"159"
    assert m[::2][1::2].tobytes() == b"37"
    assert m[::-1].tobytes() == b"987654321"
    assert m[::-2].tobytes() == b"97531"


test_slice()


def test_resizable():
    b = bytearray(b"123")
    b.append(4)
    m = memoryview(b)
    assert_raises(BufferError, lambda: b.append(5))
    m.release()
    b.append(6)
    m2 = memoryview(b)
    m4 = memoryview(m2)
    assert_raises(BufferError, lambda: b.append(5))
    m3 = memoryview(m2)
    assert_raises(BufferError, lambda: b.append(5))
    m2.release()
    assert_raises(BufferError, lambda: b.append(5))
    m3.release()
    m4.release()
    b.append(7)


test_resizable()


def test_delitem():
    a = b"abc"
    b = memoryview(a)
    assert_raises(TypeError, lambda: b.__delitem__())
    assert_raises(TypeError, lambda: b.__delitem__(0))
    assert_raises(TypeError, lambda: b.__delitem__(10))
    a = bytearray(b"abc")
    b = memoryview(a)
    assert_raises(TypeError, lambda: b.__delitem__())
    assert_raises(TypeError, lambda: b.__delitem__(1))
    assert_raises(TypeError, lambda: b.__delitem__(12))


test_delitem()


def test_empty_view_offset():
    # An empty view keeps the offset slicing left it, which can sit outside the
    # exporter, and reaches no byte through it.
    ba = bytearray(range(17))
    assert bytes(memoryview(ba)[::-9][-30::-9]) == b""
    assert bytes(memoryview(ba)[-30::-1]) == b""
    v = memoryview(ba)[::-9][-30::-9]
    assert v.shape == (0,)
    assert v.strides == (81,)
    assert v.suboffsets == ()
    b24 = bytearray(range(24))
    assert bytes(memoryview(b24).cast("B", [4, 6])[-30::-1]) == b""


test_empty_view_offset()


def test_exported_suboffsets():
    mv = memoryview(bytearray(b"abcdef"))[::-1]
    exported = mv.__buffer__(284)
    assert exported.suboffsets == ()
    assert bytes(exported) == b"fedcba"
    assert (
        bytes(memoryview(memoryview(bytearray(b"abcdefg"))[::2].__buffer__(284)))
        == b"aceg"
    )


test_exported_suboffsets()


def test_setitem_slice_strided_source():
    src = bytearray(b"abcdef")
    dst = bytearray(b"......")
    memoryview(dst)[:] = memoryview(src)[::-1]
    assert bytes(dst) == b"fedcba"
    dst = bytearray(b"...")
    memoryview(dst)[:] = memoryview(src)[::2]
    assert bytes(dst) == b"ace"


test_setitem_slice_strided_source()


def test_zero_dim_position():
    z = memoryview(bytearray(range(8)))[4:5].cast("B", [])
    assert z[()] == 4
    assert z.tolist() == 4
    w = bytearray(range(8))
    memoryview(w)[4:5].cast("B", [])[()] = 99
    assert w[4] == 99
    assert w[0] == 0


test_zero_dim_position()


def test_cast_zero_dim_size():
    assert_raises(TypeError, lambda: memoryview(bytearray(range(8))).cast("B", []))
    assert memoryview(bytearray(b"a")).cast("B", []).nbytes == 1


test_cast_zero_dim_size()


def test_hash_format():
    assert_raises(ValueError, lambda: hash(memoryview(b"abcd").cast("I")))
    hash(memoryview(b"abcd").cast("b"))
    hash(memoryview(b"abcdef")[::2])
    hash(memoryview(b"a").cast("B", []))


test_hash_format()


def test_cast_keeps_exports():
    ba = bytearray(b"abc")
    mv = memoryview(ba)
    cast = mv.cast("B")
    mv.release()
    assert_raises(BufferError, lambda: ba.clear())
    cast.release()
    ba.clear()
    assert bytes(ba) == b""


test_cast_keeps_exports()


def test_setitem_converts_before_writing():
    ba = bytearray(b"abc")
    mv = memoryview(ba)

    class Idx:
        def __index__(self):
            return len(bytes(ba))

    mv[0] = Idx()
    assert bytes(ba) == b"\x03bc"


test_setitem_converts_before_writing()


def test_pep688_exporter_aliasing():
    def exporter(view_factory):
        class C:
            def __buffer__(self, flags):
                return view_factory()

            def __release_buffer__(self, view):
                pass

        return C()

    ba = bytearray(b"abc")
    memoryview(ba)[:] = exporter(lambda: memoryview(ba))
    assert bytes(ba) == b"abc"

    ba = bytearray(b"abcdef")
    memoryview(ba)[0:3] = exporter(lambda: memoryview(ba)[3:6])
    assert bytes(ba) == b"defdef"

    ba = bytearray(b"abcdef")
    memoryview(ba)[3:6] = exporter(lambda: memoryview(ba)[0:3])
    assert bytes(ba) == b"abcabc"

    ba = bytearray(b"abcdef")
    memoryview(ba)[:] = exporter(lambda: memoryview(ba)[::-1])
    assert bytes(ba) == b"fedcba"

    ba = bytearray(b"abcdef")
    memoryview(ba)[::2] = exporter(lambda: memoryview(ba)[0:3])
    assert bytes(ba) == b"abbdcf"

    ba = bytearray(b"abcdef")
    mv = memoryview(exporter(lambda: memoryview(ba)))
    mv[:] = exporter(lambda: memoryview(ba))
    assert bytes(ba) == b"abcdef"
    mv[:] = ba
    assert bytes(ba) == b"abcdef"


test_pep688_exporter_aliasing()


def test_release_buffer_waits_for_last_view():
    class C(bytearray):
        calls = 0

        def __release_buffer__(self, view):
            type(self).calls += 1
            super().__release_buffer__(view)

    c = C(b"abcdef")
    a = memoryview(c)
    b = memoryview(a)
    a.release()
    assert C.calls == 0
    assert b.tobytes() == b"abcdef"
    b.release()
    assert C.calls == 1

    class D:
        n = 0

        def __init__(self):
            self.b = bytearray(b"abcdef")

        def __buffer__(self, flags):
            return memoryview(self.b)

        def __release_buffer__(self, view):
            type(self).n += 1

    d = D()
    m = memoryview(d)
    m2 = memoryview(m)
    m3 = m.cast("B")
    m.release()
    m2.release()
    assert D.n == 0
    m3.release()
    assert D.n == 1

    # Two acquisitions are two exports, each released on its own.
    D.n = 0
    d = D()
    a1 = memoryview(d)
    a2 = memoryview(d)
    a1.release()
    assert D.n == 1
    a2.release()
    assert D.n == 2


test_release_buffer_waits_for_last_view()


def test_failed_request_does_not_release():
    import inspect
    import mmap

    class M(mmap.mmap):
        calls = 0

        def __release_buffer__(self, view):
            type(self).calls += 1
            super().__release_buffer__(view)

    m = M(-1, 10, access=mmap.ACCESS_READ)
    assert_raises(BufferError, lambda: m.__buffer__(inspect.BufferFlags.WRITABLE))
    assert M.calls == 0


test_failed_request_does_not_release()


def test_request_shapes_exported_descriptor():
    import array

    a = array.array("I", [1, 2, 3])
    assert a.__buffer__(0).format == "B"
    assert a.__buffer__(28).format == "I"

    m = memoryview(a)
    b = m.__buffer__(0)
    assert (b.format, b.itemsize, b.ndim, b.shape, b.strides) == ("B", 4, 1, (3,), (4,))
    assert m.__buffer__(28).format == "I"

    b = a.__buffer__(0)
    assert b[0] == 1
    assert b.tolist() == [1, 2, 3]
    assert len(b.tobytes()) == 12
    b[0] = 9
    assert a[0] == 9

    n = memoryview(bytearray(b"abcdef" * 4)).cast("I", (2, 3))
    assert n.__buffer__(0).ndim == 1
    assert n.__buffer__(0).shape == (6,)
    assert n.__buffer__(8).ndim == 2
    assert n.__buffer__(8).format == "B"


test_request_shapes_exported_descriptor()


def test_release_during_index_conversion():
    # CHECK_RELEASED_AGAIN: the conversion that produces the value, and the one
    # that produced the index, both run Python that can release the view.
    ba = bytearray(b"abcdefgh")
    mv = memoryview(ba)

    class Writer:
        def __index__(self):
            mv.release()
            ba.clear()
            return 7

    try:
        mv[7] = Writer()
        raise AssertionError("write into a released view")
    except ValueError as e:
        assert "released memoryview" in str(e), e

    ba = bytearray(b"abcdefgh")
    mv = memoryview(ba)

    class Reader:
        def __index__(self):
            mv.release()
            ba.clear()
            return 7

    try:
        mv[Reader()]
        raise AssertionError("read from a released view")
    except ValueError as e:
        assert "released memoryview" in str(e), e

    # A release that does not resize still forbids the write.
    ba = bytearray(b"abcd")
    mv = memoryview(ba)

    class Quiet:
        def __index__(self):
            mv.release()
            return 65

    try:
        mv[0] = Quiet()
        raise AssertionError("write into a released view")
    except ValueError as e:
        assert "released memoryview" in str(e), e
    assert bytes(ba) == b"abcd"


test_release_during_index_conversion()


def test_cast_rejects_non_native_format():
    # get_native_fmtchar
    for fmt in ["", "ii", "<i", "z"]:
        try:
            memoryview(b"").cast(fmt)
            raise AssertionError(f"cast accepted {fmt!r}")
        except ValueError as e:
            assert "native single character format" in str(e), e
    assert memoryview(bytes(8)).cast("@i").itemsize == 4


test_cast_rejects_non_native_format()


def test_release_buffer_called_twice():
    # wrap_releasebuffer
    ba = bytearray(b"abc")
    mv = memoryview(ba)
    assert ba.__release_buffer__(mv) is None
    try:
        ba.__release_buffer__(mv)
        raise AssertionError("second release accepted")
    except ValueError as e:
        assert "already been released" in str(e), e

    mv = memoryview(bytearray(b"abc"))
    try:
        bytearray(b"abc").__release_buffer__(mv)
        raise AssertionError("release by a foreign object accepted")
    except ValueError as e:
        assert "not this object" in str(e), e


test_release_buffer_called_twice()


def test_restricted_view_compares():
    # memory_richcompare reads another view where it lies rather than acquiring
    # it, so the restricted view handed to __release_buffer__ still compares.
    seen = []

    class K(bytearray):
        def __release_buffer__(self, view):
            seen.append(memoryview(b"hello") == view)
            seen.append(view == memoryview(b"hello"))

    memoryview(K(b"hello")).release()
    assert seen == [True, True], seen


test_restricted_view_compares()


def test_release_during_slice_assignment():
    # copy_single: acquiring the source runs __buffer__, which can release the
    # destination view.
    ba = bytearray(b"abcd")
    mv = memoryview(ba)

    class Src:
        def __buffer__(self, flags):
            mv.release()
            return memoryview(b"WXYZ")

    try:
        mv[:] = Src()
        raise AssertionError("wrote through a released view")
    except ValueError as e:
        assert "released memoryview" in str(e), e
    assert bytes(ba) == b"abcd"

    # The destination of a slice assignment counts as no export, so the source
    # may resize the exporter once it has released the view.
    ba = bytearray(b"abcd")
    mv = memoryview(ba)

    class Shrink:
        def __buffer__(self, flags):
            mv.release()
            ba.clear()
            return memoryview(b"WXYZ")

    try:
        mv[:] = Shrink()
        raise AssertionError("wrote through a released view")
    except ValueError as e:
        assert "released memoryview" in str(e), e
    assert bytes(ba) == b""


test_release_during_slice_assignment()


def test_fortran_contiguity():
    # A view whose dimensions are all but one of length 1 is laid out both in
    # row-major and in column-major order.
    mv = memoryview(bytearray(range(8)))
    for shape in [(1, 8), (8, 1), (1, 1, 8), (8,)]:
        view = mv.cast("B", shape)
        assert view.c_contiguous, shape
        assert view.f_contiguous, shape
        assert view.contiguous, shape
    for shape in [(2, 4), (1, 2, 4), (2, 1, 4), (2, 2, 2)]:
        view = mv.cast("B", shape)
        assert view.c_contiguous, shape
        assert not view.f_contiguous, shape
        assert view.contiguous, shape

    # A view with no elements is laid out both ways whatever its shape.
    empty = memoryview(b"").cast("B")
    assert empty.c_contiguous and empty.f_contiguous and empty.contiguous

    scalar = memoryview(b"a").cast("B", ())
    assert scalar.c_contiguous and scalar.f_contiguous and scalar.contiguous

    strided = memoryview(bytearray(b"abcdefgh"))[::2]
    assert not strided.c_contiguous
    assert not strided.f_contiguous
    assert not strided.contiguous


test_fortran_contiguity()


def test_cast_arguments():
    # cast() takes a native single character format, optionally '@'-prefixed;
    # a zero-size format used to reach a division by zero.
    assert memoryview(b"abcd").cast("@i").itemsize == 4
    for fmt in ("0s", "4s", "<i", "", "ss"):
        assert_raises(ValueError, lambda fmt=fmt: memoryview(b"abcd").cast(fmt))

    # every element of shape is an int > 0; a 0 used to divide by zero while
    # checking the product against SSIZE_MAX
    for shape in ([0], [0, 4], [4, 0], [-1, 4], [0, 0]):
        assert_raises(
            ValueError, lambda shape=shape: memoryview(b"abcd").cast("B", shape)
        )

    class Index:
        def __index__(self):
            return 4

    for shape in ([2.0, 2], [Index()], ["4"]):
        assert_raises(
            TypeError, lambda shape=shape: memoryview(b"abcd").cast("B", shape)
        )

    assert memoryview(b"abcd").cast("B", [True, 4]).tolist() == [[97, 98, 99, 100]]


test_cast_arguments()


def test_negative_stride():
    # A reversed view starts at its last byte, so walking it from there runs
    # off the front of the exported slice.
    assert memoryview(b"dcba") == memoryview(b"abcd")[::-1]
    assert memoryview(b"abcd")[::-1] == memoryview(b"dcba")
    assert not memoryview(b"abcd") == memoryview(b"abcd")[::-1]

    b = bytearray(b"____")
    memoryview(b)[0:4] = memoryview(b"abcd")[::-1]
    assert b == bytearray(b"dcba"), b

    a = array.array("i", [1, 2, 3])
    assert memoryview(array.array("i", [3, 2, 1])) == memoryview(a)[::-1]
    assert memoryview(a)[::-1].tolist() == [3, 2, 1]


test_negative_stride()


def test_write_through_same_object():
    # Reading the source and writing the destination lock the same object
    # when they overlap, and converting a value runs Python that can reach it.
    b = bytearray(b"abcd")
    memoryview(b)[0:4] = b
    assert b == bytearray(b"abcd"), b

    b = bytearray(b"abcd")
    memoryview(b)[0:4] = memoryview(b)[::-1]
    assert b == bytearray(b"dcba"), b

    b = bytearray(b"abcd")
    memoryview(b)[0:2] = memoryview(b)[2:4]
    assert b == bytearray(b"cdcd"), b

    b = bytearray(b"abcd")
    view = memoryview(b)

    class Index:
        def __index__(self):
            view[1] = 66
            return 65

    view[0] = Index()
    assert b == bytearray(b"ABcd"), b


test_write_through_same_object()


def test_cast_between_non_byte_formats():
    # A cast re-divides bytes into items; going from one item type straight to
    # another would reinterpret what is already there.
    view = memoryview(b"abcd").cast("i")
    for fmt in ("h", "i", "f"):
        try:
            view.cast(fmt)
        except TypeError as e:
            assert "cannot cast between two non-byte formats" in str(e), e
        else:
            raise AssertionError(f"expected TypeError for cast to {fmt!r}")

    # Either side being bytes is allowed.
    assert view.cast("B").tolist() == [97, 98, 99, 100]
    assert view.cast("b").format == "b"
    assert view.cast("c").tolist() == [b"a", b"b", b"c", b"d"]
    assert memoryview(b"abcd").cast("c").cast("i").format == "i"


def test_cast_to_zero_dim():
    # A zero-dimensional view holds exactly one item, so the buffer has to be
    # that one item and no more.
    assert memoryview(b"abcd").cast("I", shape=()).tobytes() == b"abcd"
    assert memoryview(b"a").cast("B", shape=()).tobytes() == b"a"

    for source, fmt in ((b"abcd", "B"), (b"abcdefgh", "I"), (b"ab", "b")):
        try:
            memoryview(source).cast(fmt, shape=())
        except TypeError as e:
            assert "product(shape) * itemsize != buffer size" in str(e), e
        else:
            raise AssertionError(f"expected TypeError for {source!r} as {fmt!r}")


def test_hash_restricted_to_byte_formats():
    # The hash is over the bytes, so it agrees with the hash of those bytes
    # only where an item is a byte.
    data = b"abcdefgh"
    assert hash(memoryview(data)) == hash(data)
    assert hash(memoryview(data).cast("c")) == hash(data)
    assert hash(memoryview(data).cast("b")) == hash(data)

    for fmt in ("I", "i", "h", "d"):
        try:
            hash(memoryview(data).cast(fmt))
        except ValueError as e:
            assert "hashing is restricted to formats" in str(e), e
        else:
            raise AssertionError(f"expected ValueError for format {fmt!r}")


def test_tobytes_order():
    view = memoryview(b"abcdefgh")
    for order in (None, "C", "F", "A"):
        assert view.tobytes(order=order) == b"abcdefgh", order

    # A multidimensional view is laid out C-contiguously, so a Fortran-ordered
    # copy walks it down the columns instead.
    grid = memoryview(b"abcdefgh").cast("B", shape=(2, 4))
    assert grid.tolist() == [[97, 98, 99, 100], [101, 102, 103, 104]]
    assert grid.tobytes() == b"abcdefgh"
    assert grid.tobytes(order="C") == b"abcdefgh"
    assert grid.tobytes(order="A") == b"abcdefgh"
    assert grid.tobytes(order="F") == b"aebfcgdh"

    cube = memoryview(b"abcdefgh").cast("B", shape=(2, 2, 2))
    assert cube.tobytes(order="F") == b"aecgbfdh"

    for order in ("Z", "c", "f", ""):
        try:
            view.tobytes(order=order)
        except ValueError as e:
            assert str(e) == "order must be 'C', 'F' or 'A'", e
        else:
            raise AssertionError(f"expected ValueError for order {order!r}")


test_cast_between_non_byte_formats()
test_cast_to_zero_dim()
test_hash_restricted_to_byte_formats()
test_tobytes_order()


def test_index_key_goes_through_index():
    class I:
        def __index__(self):
            return 1

    c = memoryview(bytes(range(24))).cast("B", shape=(4, 6))
    assert c[I(), 2] == 8
    assert c[1, I()] == 7
    assert memoryview(b"\x00\x01")[(I(),)] == 1

    w = memoryview(bytearray(range(24))).cast("B", shape=(4, 6))
    w[I(), 2] = 99
    assert w[1, 2] == 99

    # What the key is follows from its types, so an item that answers
    # __index__ but raises reports that rather than the key being invalid.
    class Boom:
        def __index__(self):
            raise RuntimeError("boom")

    for key in [Boom(), (Boom(), 0), (0, Boom())]:
        try:
            c[key]
            raise AssertionError("no error")
        except RuntimeError as e:
            assert str(e) == "boom", e

    try:
        c[object(), 0]
        raise AssertionError("no error")
    except TypeError as e:
        assert "invalid slice key" in str(e), e


test_index_key_goes_through_index()


def test_index_error_names_the_dimension():
    c = memoryview(bytearray(range(24))).cast("B", shape=(2, 3, 4))
    for key, dimension in [((0, 3, 0), 2), ((0, 0, -5), 3), ((2, 0, 0), 1)]:
        try:
            c[key]
            raise AssertionError("no error")
        except IndexError as e:
            assert str(e) == f"index out of bounds on dimension {dimension}", e

    try:
        memoryview(bytearray(range(4)))[100]
        raise AssertionError("no error")
    except IndexError as e:
        assert str(e) == "index out of bounds on dimension 1", e


test_index_error_names_the_dimension()


def test_release_refuses_while_exported():
    ba = bytearray(b"abc")
    mv = memoryview(ba)
    exported = mv.__buffer__(0)
    for release in [lambda: mv.release(), lambda: mv.__exit__()]:
        try:
            release()
            raise AssertionError("released while exported")
        except BufferError as e:
            assert str(e) == "memoryview has 1 exported buffer", e
    del exported
    mv.release()
    mv.release()


test_release_refuses_while_exported()


def test_hash_asks_the_exporter():
    # A view is no more hashable than what it looks at.
    assert hash(memoryview(b"abc")) == hash(b"abc")
    try:
        hash(memoryview(bytearray(b"abc")).toreadonly())
        raise AssertionError("hashed a view on an unhashable exporter")
    except TypeError as e:
        assert "unhashable type: 'bytearray'" in str(e), e

    # Releasing the view from inside that hash is refused rather than obeyed.
    class E(bytes):
        def __hash__(self):
            mv.release()
            return 123

    mv = memoryview(E(b"abcd"))
    try:
        hash(mv)
        raise AssertionError("released the view being hashed")
    except BufferError as e:
        assert str(e) == "memoryview has 1 exported buffer", e


test_hash_asks_the_exporter()


def test_hex_measures_the_separator():
    # The separator is measured the way len() measures it, and read where it lies.
    class One(bytes):
        def __len__(self):
            return 1

    class Two(bytes):
        def __len__(self):
            return 2

    for target in [b"abcd", bytearray(b"abcd"), memoryview(b"abcd")]:
        assert target.hex(One(b"::")) == "61:62:63:64"
        assert target.hex(b":") == "61:62:63:64"
        # An object claiming a length it does not have separates with NUL.
        assert target.hex(One(b"")) == "61\x0062\x0063\x0064"
        for bad in [Two(b":"), b"::"]:
            try:
                target.hex(bad)
                raise AssertionError("no error")
            except ValueError as e:
                assert str(e) == "sep must be length 1.", e
        # The separator is checked before anything is written out.
        try:
            target.hex(b"::", 0)
            raise AssertionError("no error")
        except ValueError as e:
            assert str(e) == "sep must be length 1.", e

    assert b"".hex(b":") == ""
    try:
        b"".hex(b"::")
        raise AssertionError("no error")
    except ValueError as e:
        assert str(e) == "sep must be length 1.", e

    # Releasing the view from inside that measurement is refused.
    ba = bytearray(b"A" * 8)
    mv = memoryview(ba)

    class S(bytes):
        def __len__(self):
            mv.release()
            return 1

    try:
        mv.hex(S(b":"))
        raise AssertionError("released the view being written out")
    except BufferError as e:
        assert str(e) == "memoryview has 1 exported buffer", e


test_hex_measures_the_separator()


def test_cast_bounds_the_dimensions():
    mv = memoryview(bytearray(range(8)))
    assert mv.cast("B", (1,) * 63 + (8,)).ndim == 64
    for shape in [(1,) * 64 + (8,), (1,) * 99 + (8,), [1] * 64 + [8]]:
        try:
            mv.cast("B", shape)
            raise AssertionError("cast past the limit")
        except ValueError as e:
            assert str(e) == "memoryview: number of dimensions must not exceed 64", e

    # The limit is answered before the shape is looked at any further.
    try:
        mv.cast("B", (2, 4)).cast("B", (1,) * 64 + (8,))
        raise AssertionError("cast past the limit")
    except ValueError as e:
        assert str(e) == "memoryview: number of dimensions must not exceed 64", e


test_cast_bounds_the_dimensions()


def test_setitem_error_kinds():
    import array

    # A value the format has no room for and a value of the wrong kind are
    # different errors, the way they are for any other conversion.
    for fmt, over, under in (
        ("B", 300, -1),
        ("b", 128, -129),
        ("i", 2**31, -(2**31) - 1),
    ):
        view = memoryview(array.array(fmt, [0, 0]))
        for value in (over, under):
            try:
                view[0] = value
            except ValueError as e:
                assert str(e) == f"memoryview: invalid value for format '{fmt}'", e
            else:
                raise AssertionError(f"expected ValueError for {fmt!r} {value}")
        for value in ("x", 1.5, None, [1]):
            try:
                view[0] = value
            except TypeError as e:
                assert str(e) == f"memoryview: invalid type for format '{fmt}'", e
            else:
                raise AssertionError(f"expected TypeError for {fmt!r} {value!r}")

    # A bytes item is a value error when it is the wrong length.
    chars = memoryview(bytearray(b"ab")).cast("c")
    for value in (b"", b"xy"):
        try:
            chars[0] = value
        except ValueError as e:
            assert str(e) == "memoryview: invalid value for format 'c'", e
        else:
            raise AssertionError(f"expected ValueError for {value!r}")


def test_setitem_propagates_index_errors():
    # An error raised by the value's own code is the answer, not a report that
    # the value was the wrong kind.
    class Boom:
        def __index__(self):
            raise ZeroDivisionError("boom")

    class NotAnInt:
        def __index__(self):
            return "not an int"

    view = memoryview(bytearray(b"ab"))
    try:
        view[0] = Boom()
    except ZeroDivisionError as e:
        assert str(e) == "boom", e
    else:
        raise AssertionError("expected ZeroDivisionError")

    try:
        view[0] = NotAnInt()
    except TypeError as e:
        assert str(e) == "memoryview: invalid type for format 'B'", e
    else:
        raise AssertionError("expected TypeError")


def test_delete_answers_readonly_first():
    # Nothing can be deleted from a memoryview, but what cannot be written
    # says so first.
    try:
        del memoryview(b"ab")[0]
    except TypeError as e:
        assert str(e) == "cannot modify read-only memory", e
    else:
        raise AssertionError("expected TypeError")

    view = memoryview(bytearray(b"abcd"))
    for needle in (0, slice(0, 2)):
        try:
            del view[needle]
        except TypeError as e:
            assert str(e) == "cannot delete memory", e
        else:
            raise AssertionError(f"expected TypeError for {needle!r}")


test_setitem_error_kinds()
test_setitem_propagates_index_errors()
test_delete_answers_readonly_first()


def test_bool_format_keeps_its_own_error():
    # Deciding truth is the value's own code, and what it raises is the answer.
    class Raises:
        def __init__(self, exc):
            self.exc = exc

        def __bool__(self):
            raise self.exc

    view = memoryview(bytearray(b"\x00")).cast("?")
    for exc in (ZeroDivisionError("boom"), ValueError("nope"), TypeError("nah")):
        try:
            view[0] = Raises(exc)
        except type(exc) as e:
            assert str(e) == str(exc), e
        else:
            raise AssertionError(f"expected {type(exc).__name__}")


test_bool_format_keeps_its_own_error()
