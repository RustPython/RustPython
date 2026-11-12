import sys


class Callable:
    def __init__(self):
        self.count = 0

    def __call__(self):
        self.count += 1
        return self.count


c = Callable()
assert 1 == c()
assert 2 == c()


class Inherited(Callable):
    pass


i = Inherited()

assert 1 == i()


# CALL_FUNCTION_EX consumes argument containers for exact Python functions,
# but keeps them alive while invoking bound methods and other callables.
call_events = []


class CallItem:
    def __init__(self, name):
        self.name = name

    def __del__(self):
        call_events.append(self.name)

    def __str__(self):
        return self.name


def consume_call_item(x, fail=False):
    del x
    call_events.append("body")
    if fail:
        raise ValueError


class CallConsumer:
    def method(self, x, fail=False):
        del x
        call_events.append("body")
        if fail:
            raise ValueError

    __call__ = method


class CallArgs(tuple):
    def __del__(self):
        call_events.append("args")


def generate_call_args():
    yield CallItem("x")
    call_events.append("exhausted")


for consume, is_function, is_method in (
    (consume_call_item, True, False),
    (CallConsumer().method, False, True),
    (CallConsumer(), False, False),
):
    kinds = ("keywords", "tuple_subclass", "generator")
    if is_function or is_method:
        kinds = ("direct", *kinds)
    for fail in (False, True):
        for kind in kinds:
            # Exercise both cold and specialized call sites.
            for _ in range(32):
                call_events.clear()
                raised = False
                try:
                    if kind == "direct":
                        consume(x=CallItem("x"), fail=fail)
                    elif kind == "keywords":
                        consume(**dict(x=CallItem("x"), fail=fail))
                    elif kind == "tuple_subclass":
                        consume(*CallArgs((CallItem("x"),)), fail=fail)
                    else:
                        consume(*generate_call_args(), fail=fail)
                except ValueError:
                    raised = True
                assert raised == fail
                expected = []
                if kind == "tuple_subclass":
                    expected.append("args")
                elif kind == "generator":
                    expected.append("exhausted")
                if is_function or (is_method and kind == "direct"):
                    expected.extend(("x", "body"))
                else:
                    expected.extend(("body", "x"))
                assert call_events == expected, (kind, fail, call_events)


class ReplacingCallArgs(CallArgs):
    def __iter__(self):
        call_events.append("iter")
        return iter((CallItem("replacement"),))


# Tuple subclasses must be normalized through iteration, not copied as exact
# tuples. Their original members can die before the replacement is passed on.
for consume, is_function in (
    (consume_call_item, True),
    (CallConsumer().method, False),
):
    call_events.clear()
    consume(*ReplacingCallArgs((CallItem("original"),)))
    expected = ["iter", "args", "original"]
    if is_function:
        expected.extend(("replacement", "body"))
    else:
        expected.extend(("body", "replacement"))
    assert call_events == expected, call_events


class CallSeparator(str):
    def __del__(self):
        call_events.append(str(self))


class CallSink:
    def write(self, text):
        # All keyword values must survive throughout the native invocation.
        assert not call_events, call_events

    def __del__(self):
        call_events.append("file")


# Native CALL_FUNCTION_EX releases keywords in insertion order, followed by
# positional values in reverse tuple order, on both success and conversion error.
call_events.clear()
print(
    *(CallItem("a"), CallItem("b")),
    **dict(sep=CallSeparator("sep"), end=CallSeparator("end"), file=CallSink()),
)
assert call_events == ["sep", "end", "file", "b", "a"], call_events

call_events.clear()
try:
    print(
        *(CallItem("a"), CallItem("b")),
        **dict(sep=CallItem("sep"), end=CallItem("end"), file=CallSink()),
    )
except TypeError:
    pass
else:
    raise AssertionError("print accepted a non-string separator")
assert call_events == ["sep", "end", "file", "b", "a"], call_events


# Instrumented CALL_FUNCTION_EX retains the containers even for an exact
# Python function. Disabling instrumentation restores ordinary ownership.
def monitored_consume_call_item(fail):
    consume_call_item(**dict(x=CallItem("x"), fail=fail))


monitoring = sys.monitoring
monitoring.use_tool_id(2, "call-argument-ownership")
try:
    monitoring.register_callback(2, monitoring.events.CALL, lambda *args: None)
    for enabled in (False, True, False):
        monitoring.set_local_events(
            2,
            monitored_consume_call_item.__code__,
            monitoring.events.CALL if enabled else 0,
        )
        for fail in (False, True):
            call_events.clear()
            raised = False
            try:
                monitored_consume_call_item(fail)
            except ValueError:
                raised = True
            assert raised == fail
            expected = ["body", "x"] if enabled else ["x", "body"]
            assert call_events == expected, (enabled, fail, call_events)
finally:
    monitoring.set_local_events(2, monitored_consume_call_item.__code__, 0)
    monitoring.register_callback(2, monitoring.events.CALL, None)
    monitoring.free_tool_id(2)
