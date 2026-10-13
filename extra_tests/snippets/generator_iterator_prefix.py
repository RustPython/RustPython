"""CPython 3.15 iterator-pair, generator-creation, and SEND regression checks."""


def test_iterator_pairs_survive_specialized_loops_and_unwinding():
    class Iterator:
        def __init__(self):
            self.index = 0

        def __iter__(self):
            return self

        def __next__(self):
            if self.index == 4:
                raise StopIteration
            value = self.index
            self.index += 1
            return value

    def values():
        yield from range(4)

    def consume(items):
        total = 0
        for item in items:
            total += item
        return total

    def leave(items):
        for item in items:
            for inner in range(3):
                if inner == 1:
                    return item + inner
        return -1

    for warmup in range(200):
        assert consume(range(4)) == 6
        assert consume([0, 1, 2, 3]) == 6
        assert consume((0, 1, 2, 3)) == 6
        assert consume(values()) == 6
        assert consume(Iterator()) == 6
        assert leave(range(4)) == 1
        assert [x * y for x in range(3) for y in range(2)] == [0, 0, 0, 1, 0, 2]
        for x in range(5):
            if x == 2:
                break
        assert x == 2


def test_eager_generator_prefix_preserves_laziness_and_created_state():
    events = []

    class Iterable:
        def __iter__(self):
            events.append("iter")
            return iter((4, 5))

    def body(value):
        events.append(value)
        return value

    g = (body(value) for value in Iterable())
    assert events == ["iter"]
    assert not g.gi_suspended
    try:
        g.send(1)
    except TypeError:
        pass
    else:
        raise AssertionError("created generator accepted non-None")
    assert events == ["iter"]
    assert next(g) == 4
    assert events == ["iter", 4]
    assert g.gi_suspended
    assert list(g) == [5]

    class BadIterable:
        def __iter__(self):
            raise ValueError("eager")

    try:
        (value for value in BadIterable())
    except ValueError as error:
        assert error.args == ("eager",)
    else:
        raise AssertionError("iterator acquisition was deferred")

    def outer(value):
        def generator():
            events.append("body")
            yield value

        return generator()

    g = outer(17)
    assert "body" not in events
    g.close()
    assert "body" not in events
    assert list(outer(19)) == [19]


def test_send_pairs_support_delegation_and_throw_cleanup():
    def leaf():
        sent = yield 1
        return sent

    def chain(depth):
        if depth:
            return (yield from chain(depth - 1))
        return (yield from leaf())

    for warmup in range(200):
        g = chain(12)
        assert next(g) == 1
        assert g.gi_yieldfrom is not None
        try:
            g.send(17)
        except StopIteration as error:
            assert error.value == 17
        else:
            raise AssertionError("delegation did not finish")

    class Delegate:
        def __iter__(self):
            return self

        def __next__(self):
            return 3

        def send(self, value):
            return value

        def throw(self, error):
            raise StopIteration(23)

    def custom():
        result = yield from Delegate()
        yield result

    g = custom()
    assert next(g) == 3
    assert g.send(9) == 9
    assert g.throw(ValueError) == 23
    assert list(g) == []

    class RaisingDelegate(Delegate):
        def throw(self, error):
            raise RuntimeError("delegate")

    def raises():
        try:
            yield from RaisingDelegate()
        except RuntimeError:
            yield "caught"

    g = raises()
    assert next(g) == 3
    assert g.throw(RuntimeError) == "caught"
    assert list(g) == []

    events = []

    def close_leaf():
        try:
            yield 1
        finally:
            events.append("closed")

    def close_outer():
        yield from close_leaf()

    g = close_outer()
    assert next(g) == 1
    g.close()
    assert events == ["closed"]


def test_await_and_async_generator_expression_keep_iterator_shapes():
    events = []

    class AsyncIterator:
        def __init__(self):
            self.index = 0

        def __aiter__(self):
            events.append("aiter")
            return self

        async def __anext__(self):
            if self.index == 3:
                raise StopAsyncIteration
            value = self.index
            self.index += 1
            return value

    g = (value * 2 async for value in AsyncIterator())
    assert events == ["aiter"]
    assert not g.ag_running

    async def consume():
        result = []
        async for value in g:
            result.append(value)
        return result

    coro = consume()
    try:
        coro.send(None)
    except StopIteration as error:
        assert error.value == [0, 2, 4]
    else:
        raise AssertionError("unexpected coroutine suspension")
    assert events == ["aiter"]

    class Awaitable:
        def __await__(self):
            sent = yield "pause"
            return sent

    async def awaits():
        return await Awaitable()

    for warmup in range(200):
        coro = awaits()
        assert coro.send(None) == "pause"
        assert coro.cr_await is not None
        try:
            coro.send(31)
        except StopIteration as error:
            assert error.value == 31
        else:
            raise AssertionError("await did not finish")


test_iterator_pairs_survive_specialized_loops_and_unwinding()
test_eager_generator_prefix_preserves_laziness_and_created_state()
test_send_pairs_support_delegation_and_throw_cleanup()
test_await_and_async_generator_expression_keep_iterator_shapes()


def test_eager_prefix_is_untraceable_but_reports_monitoring_errors():
    import sys

    trace = []

    class Good:
        def __iter__(self):
            return iter([1])

    class Bad:
        def __iter__(self):
            raise ValueError("eager iterator")

    def create(iterable):
        return (value for value in iterable)

    def tracer(frame, event, arg):
        if frame.f_code.co_name in ("create", "<genexpr>", "__iter__"):
            trace.append((frame.f_code.co_name, event))
        return tracer

    sys.settrace(tracer)
    try:
        generator = create(Good())
        generator.close()
        try:
            create(Bad())
        except ValueError as error:
            names = []
            traceback = error.__traceback__
            while traceback is not None:
                names.append(traceback.tb_frame.f_code.co_name)
                traceback = traceback.tb_next
        else:
            raise AssertionError("expected eager iterator error")
    finally:
        sys.settrace(None)
    assert not any(name == "<genexpr>" for name, event in trace)
    assert trace == [
        ("create", "call"),
        ("create", "line"),
        ("__iter__", "call"),
        ("__iter__", "line"),
        ("__iter__", "return"),
        ("create", "return"),
        ("create", "call"),
        ("create", "line"),
        ("__iter__", "call"),
        ("__iter__", "line"),
        ("__iter__", "exception"),
        ("__iter__", "return"),
        ("create", "exception"),
        ("create", "return"),
        ("create", "exception"),
        ("create", "return"),
    ]
    assert ("__iter__", "call") in trace
    assert ("__iter__", "exception") in trace
    assert names == [
        "test_eager_prefix_is_untraceable_but_reports_monitoring_errors",
        "create",
        "__iter__",
    ]

    monitoring = sys.monitoring
    records = []
    event_names = (
        "PY_START",
        "PY_RESUME",
        "PY_UNWIND",
        "RAISE",
        "LINE",
        "INSTRUCTION",
    )
    monitoring.use_tool_id(2, "iterator-contract-315")
    try:
        for name in event_names:
            event = getattr(monitoring.events, name)

            def callback(code, *args, name=name):
                if code.co_name == "<genexpr>":
                    records.append(name)

            monitoring.register_callback(2, event, callback)
        monitoring.set_events(
            2, sum(getattr(monitoring.events, name) for name in event_names)
        )
        generator = create(Good())
        generator.close()
        assert records == []
        try:
            create(Bad())
        except ValueError:
            pass
        else:
            raise AssertionError("expected monitored eager iterator error")
    finally:
        monitoring.set_events(2, 0)
        monitoring.free_tool_id(2)
    assert records == ["RAISE", "PY_UNWIND"]


test_eager_prefix_is_untraceable_but_reports_monitoring_errors()


def test_created_generator_frames_do_not_retain_materialized_caller():
    import sys

    def generator():
        yield 1

    async def coroutine():
        return 1

    async def async_generator():
        yield 1

    def create():
        caller = sys._getframe()
        return (
            (value for value in [1]),
            generator(),
            coroutine(),
            async_generator(),
            caller,
        )

    genexpr, gen, coro, agen, caller = create()
    assert caller.f_code.co_name == "create"
    assert genexpr.gi_frame.f_back is None
    assert gen.gi_frame.f_back is None
    assert coro.cr_frame.f_back is None
    assert agen.ag_frame.f_back is None
    genexpr.close()
    gen.close()
    coro.close()
    close = agen.aclose()
    try:
        close.send(None)
    except StopIteration:
        pass
    else:
        raise AssertionError("unstarted async generator close suspended")


test_created_generator_frames_do_not_retain_materialized_caller()


def test_instrumented_iteration_and_send_keep_both_iterator_slots():
    import sys

    def iterate(values):
        result = []
        for value in values:
            result.append(value)
        return result

    def leaf():
        yield 1
        return 9

    def delegate():
        return (yield from leaf())

    monitoring = sys.monitoring
    records = []
    event_names = ("BRANCH_LEFT", "BRANCH_RIGHT", "STOP_ITERATION", "PY_YIELD")
    monitoring.use_tool_id(2, "instrumented-iterator-contract-315")
    try:
        for name in event_names:
            event = getattr(monitoring.events, name)

            def callback(code, *args, name=name):
                if code.co_name in ("iterate", "delegate"):
                    records.append((code.co_name, name))

            monitoring.register_callback(2, event, callback)
        monitoring.set_events(
            2, sum(getattr(monitoring.events, name) for name in event_names)
        )
        assert iterate(range(3)) == [0, 1, 2]
        assert iterate([0, 1, 2]) == [0, 1, 2]
        assert iterate((0, 1, 2)) == [0, 1, 2]
        assert iterate(value for value in range(3)) == [0, 1, 2]
        generator = delegate()
        assert next(generator) == 1
        try:
            next(generator)
        except StopIteration as error:
            assert error.value == 9
        else:
            raise AssertionError("instrumented delegate did not finish")
    finally:
        monitoring.set_events(2, 0)
        monitoring.free_tool_id(2)
    assert ("iterate", "BRANCH_LEFT") in records
    assert ("iterate", "BRANCH_RIGHT") in records
    assert ("delegate", "PY_YIELD") in records
    assert ("delegate", "STOP_ITERATION") in records


test_instrumented_iteration_and_send_keep_both_iterator_slots()


def test_eager_iterator_frame_inspection_skips_incomplete_generator():
    import sys

    iterator_frames = []
    caller_frames = []
    seen = []

    class Iterable:
        def __iter__(self):
            frame = sys._getframe()
            caller = sys._getframe(1)
            iterator_frames.append(frame)
            caller_frames.append(caller)
            seen.append((caller.f_code.co_name, frame.f_back.f_code.co_name))
            return iter([7])

    def create():
        return (value for value in Iterable())

    generator = create()
    assert seen == [("create", "create")]
    # Both real frames have returned; retained f_back must still refer to
    # the complete creator rather than its hidden generator preamble.
    assert iterator_frames[0].f_back is caller_frames[0]
    assert iterator_frames[0].f_back.f_code.co_name == "create"
    assert generator.gi_frame.f_back is None
    assert list(generator) == [7]


test_eager_iterator_frame_inspection_skips_incomplete_generator()


def test_unstarted_coroutine_warning_survives_creation_preamble():
    import gc
    import warnings

    async def pending():
        return 7

    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always", RuntimeWarning)
        coroutine = pending()
        del coroutine
        gc.collect()
    assert len(caught) == 1
    assert caught[0].category is RuntimeWarning
    assert str(caught[0].message).endswith("pending' was never awaited")

    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always", RuntimeWarning)
        coroutine = pending()
        coroutine.close()
        del coroutine
        gc.collect()
    assert caught == []


test_unstarted_coroutine_warning_survives_creation_preamble()
