import operator

from testutils import assert_raises

for concat in (operator.add, operator.concat, bytearray.__add__):
    target = bytearray(b"a")
    events = []

    class ReentrantBuffer:
        def __buffer__(self, flags):
            events.append("acquire")
            with assert_raises(BufferError):
                target.__init__(object())
            with assert_raises(BufferError):
                target.append(120)
            target[0] = ord("A")
            return memoryview(b"b")

        def __release_buffer__(self, view):
            events.append("release")
            # The left operand's export must already have been released.
            target.append(99)

    assert concat(target, ReentrantBuffer()) == bytearray(b"Ab")
    assert events == ["acquire", "release"]
    assert target == bytearray(b"Ac")
    target.append(100)

    class FailingBuffer:
        def __buffer__(self, flags):
            raise ValueError("buffer acquisition failed")

    with assert_raises(TypeError) as error:
        concat(target, FailingBuffer())
    assert str(error.exception) == "can't concat FailingBuffer to bytearray"
    target.append(101)

    events = []

    class Left(bytearray):
        def __buffer__(self, flags):
            events.append("left acquire")
            return memoryview(b"L")

        def __release_buffer__(self, view):
            events.append("left release")

    class Right:
        def __buffer__(self, flags):
            events.append("right acquire")
            return memoryview(b"R")

        def __release_buffer__(self, view):
            events.append("right release")

    assert concat(Left(b"ignored"), Right()) == bytearray(b"LR")
    assert events == [
        "left acquire",
        "right acquire",
        "left release",
        "right release",
    ]

    events = []
    with assert_raises(TypeError) as error:
        concat(Left(), FailingBuffer())
    assert str(error.exception) == "can't concat FailingBuffer to Left"
    assert events == ["left acquire", "left release"]

    class FailingLeft(bytearray):
        def __buffer__(self, flags):
            events.append("left acquire")
            raise ValueError("left acquisition failed")

    events = []
    with assert_raises(TypeError) as error:
        concat(FailingLeft(), Right())
    assert str(error.exception) == "can't concat Right to FailingLeft"
    assert events == ["left acquire"]
