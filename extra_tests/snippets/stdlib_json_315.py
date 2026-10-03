"""Bounded JSON 3.15 contract probes; require the native accelerator throughout.

Optional first argument is a Lib overlay containing json. Each test runs with
unchanged assertions under CPython and the selected saved RustPython binary.
"""

import sys

if len(sys.argv) > 1:
    sys.path.insert(0, sys.argv[1])

import _json
import gc
import io
import json
import weakref


def raises(kind, call):
    try:
        call()
    except kind as exc:
        return exc
    raise AssertionError("Expected " + kind.__name__)


def test_accelerator_active():
    assert json.scanner.make_scanner is _json.make_scanner
    assert json.encoder.c_make_encoder is _json.make_encoder
    assert type(json.JSONDecoder().scan_once).__module__ == "_json"


def test_ordinary_json():
    text = '{"a":[1,{},[],true,null,"é"],"b":2.5}'
    expected = {"a": [1, {}, [], True, None, "é"], "b": 2.5}
    assert json.loads(text) == expected
    assert json.loads(json.dumps(expected)) == expected


def test_tuple_nested_and_empty():
    assert json.loads("[]", array_hook=tuple) == ()
    assert json.loads("[ [1], [], [2, [3]] ]", array_hook=tuple) == (
        (1,),
        (),
        (2, (3,)),
    )


def test_load_bytes_and_bytearray():
    for text in ("[1,[]]", b"[1,[]]", bytearray(b"[1,[]]")):
        assert json.loads(text, array_hook=tuple) == (1, ())
    assert json.load(io.StringIO("[1,[]]"), array_hook=tuple) == (1, ())


def test_none_hook():
    assert json.loads("[1,[]]", array_hook=None) == [1, []]


def test_none_return():
    seen = []

    def hook(values):
        seen.append(values[:])
        return None

    assert json.loads("[[],[1]]", array_hook=hook) is None
    assert seen == [[], [1], [None, None]]


def test_mutating_hook_receives_exact_list():
    seen = []

    def hook(values):
        assert type(values) is list
        seen.append(values)
        values.append("added")
        return values

    result = json.loads("[[]]", array_hook=hook)
    assert result == [["added"], "added"]
    assert result is seen[-1]
    assert result[0] is seen[0]


def test_object_hook_order():
    events = []

    def array(values):
        events.append(("array", repr(values)))
        return tuple(values)

    def obj(values):
        events.append(("object", repr(values)))
        return values

    assert json.loads('[{"x":[1,[]]},[2]]', array_hook=array, object_hook=obj) == (
        {"x": (1, ())},
        (2,),
    )
    assert events == [
        ("array", "[]"),
        ("array", "[1, ()]"),
        ("object", "{'x': (1, ())}"),
        ("array", "[2]"),
        ("array", "[{'x': (1, ())}, (2,)]"),
    ]


def test_pairs_priority_and_order():
    events = []

    def array(values):
        events.append("array")
        return tuple(values)

    def pairs(values):
        events.append("pairs")
        return frozendict(values)

    def forbidden_obj(values):
        raise AssertionError("object_pairs_hook must take priority")

    result = json.loads(
        '[{"x":[],"x":[1]},{}]',
        array_hook=array,
        object_pairs_hook=pairs,
        object_hook=forbidden_obj,
    )
    assert result == (frozendict(x=(1,)), frozendict())
    assert events == ["array", "array", "pairs", "pairs", "array"]


def test_deep_immutable():
    for keyword in ("object_hook", "object_pairs_hook"):
        result = json.loads(
            '{"x":[{"y":[]},[1]]}', array_hook=tuple, **{keyword: frozendict}
        )
        assert type(result) is frozendict
        assert type(result["x"]) is tuple
        assert type(result["x"][0]) is frozendict
        assert result == frozendict(x=(frozendict(y=()), (1,)))
        assert isinstance(hash(result), int)


def test_falsey_callable_is_not_coerced():
    class Hook:
        def __bool__(self):
            raise AssertionError("hook truth testing is forbidden")

        def __call__(self, values):
            return tuple(values)

    assert json.loads("[[]]", array_hook=Hook()) == ((),)


def test_noncallable_validation_is_deferred():
    for hook in (0, False, "no", [], {}):
        decoder = json.JSONDecoder(array_hook=hook)
        assert decoder.decode("1") == 1
        assert decoder.decode("{}") == {}
        for text in ("[]", "[1]", '{"a":[]}'):
            exc = raises(TypeError, lambda: decoder.decode(text))
            assert str(exc) == f"'{type(hook).__name__}' object is not callable"


def test_hook_exception_identity_and_recovery():
    for kind in (RuntimeError, ValueError, TypeError, StopIteration):
        error = kind("hook failure")
        events = []

        def hook(values):
            events.append(values[:])
            if not values:
                raise error
            return tuple(values)

        decoder = json.JSONDecoder(array_hook=hook)
        exc = raises(kind, lambda: decoder.scan_once("[[],[1]]", 0))
        assert exc is error
        assert events == [[]]
        assert decoder.decode("[2]") == (2,)


def test_object_exception_blocks_enclosing_array_hook():
    error = RuntimeError("object failure")
    events = []

    def array(values):
        events.append("array")
        return tuple(values)

    def obj(values):
        events.append("object")
        raise error

    exc = raises(
        RuntimeError,
        lambda: json.loads('[{"x":[]}]', array_hook=array, object_hook=obj),
    )
    assert exc is error
    assert events == ["array", "object"]


def test_unicode_raw_decode_positions():
    text = 'x["é",["𝄞"]]tail'
    result, end = json.JSONDecoder(array_hook=tuple).raw_decode(text, 1)
    assert result == ("é", ("𝄞",))
    assert end == text.index("tail")


def test_malformed_arrays_only_complete_inner_hooks():
    cases = [
        ("[]x", [()], 2),
        ("[[],]", [()], 3),
        ("[", [], 1),
        ("[1", [], 2),
        ("[1,]", [], 2),
    ]
    for text, expected, pos in cases:
        events = []

        def hook(values):
            events.append(tuple(values))
            return tuple(values)

        exc = raises(json.JSONDecodeError, lambda: json.loads(text, array_hook=hook))
        assert exc.pos == pos, (text, exc.pos, pos)
        assert events == expected, (text, events, expected)


def test_scanner_captures_readonly_hook():
    decoder = json.JSONDecoder(array_hook=tuple)
    scanner = decoder.scan_once
    assert scanner.array_hook is tuple
    decoder.array_hook = list
    assert scanner.array_hook is tuple
    assert decoder.decode("[[]]") == ((),)
    raises(AttributeError, lambda: setattr(scanner, "array_hook", list))
    raises(AttributeError, lambda: delattr(scanner, "array_hook"))
    assert json.JSONDecoder().scan_once.array_hook is None


def test_context_attribute_order():
    seen = []

    class Context:
        def __getattribute__(self, name):
            seen.append(name)
            return {
                "strict": True,
                "object_hook": None,
                "object_pairs_hook": None,
                "array_hook": tuple,
                "parse_float": float,
                "parse_int": int,
                "parse_constant": str,
            }[name]

    scanner = _json.make_scanner(Context())
    assert seen == [
        "strict",
        "object_hook",
        "object_pairs_hook",
        "array_hook",
        "parse_float",
        "parse_int",
        "parse_constant",
    ]
    assert scanner("[]", 0) == ((), 2)


def test_context_requires_array_hook():
    class Context:
        strict = True
        object_hook = None
        object_pairs_hook = None
        parse_float = float
        parse_int = int
        parse_constant = str

    exc = raises(AttributeError, lambda: _json.make_scanner(Context()))
    assert "array_hook" in str(exc)


def test_reentrant_scanner():
    def hook(values):
        if values == [1]:
            return ("inner", decoder.decode("[2]"))
        return tuple(values)

    decoder = json.JSONDecoder(array_hook=hook)
    assert decoder.decode("[[1],[]]") == (("inner", (2,)), ())


def test_hook_gc_lifetime_and_cycle():
    class Hook:
        def __call__(self, values):
            return tuple(values)

    hook = Hook()
    ref = weakref.ref(hook)
    decoder = json.JSONDecoder(array_hook=hook)
    decoder.array_hook = None
    del hook
    gc.collect()
    assert ref() is not None
    assert decoder.decode("[]") == ()
    ref().decoder = decoder
    del decoder
    gc.collect()
    assert ref() is None


def test_frozendict_encode_native():
    def no_default(obj):
        raise AssertionError("frozendict must not call default")

    value = frozendict(b=(frozendict(z=1),), a=frozendict())
    assert json.dumps(value, default=no_default) == '{"b": [{"z": 1}], "a": {}}'
    assert json.dumps(value, sort_keys=True, default=no_default) == (
        '{"a": {}, "b": [{"z": 1}]}'
    )
    assert json.dumps(value, indent=2, default=no_default) == (
        '{\n  "b": [\n    {\n      "z": 1\n    }\n  ],\n  "a": {}\n}'
    )


def test_frozendict_key_errors_and_skipkeys():
    value = frozendict({b"bad": 1, "good": 2})
    raises(TypeError, lambda: json.dumps(value))
    assert json.dumps(value, skipkeys=True) == '{"good": 2}'
    raises(TypeError, lambda: json.dumps(frozendict({1: 0, "a": 0}), sort_keys=True))


def test_frozendict_cycle():
    values = []
    value = frozendict(items=values)
    values.append(value)
    exc = raises(ValueError, lambda: json.dumps(value))
    assert str(exc) == "Circular reference detected"


def test_frozendict_subclass_items():
    class Frozen(frozendict):
        def items(self):
            return [("override", 2)]

    assert json.dumps(Frozen(real=1)) == '{"override": 2}'
    assert json.dumps(Frozen()) == "{}"


def test_dict_subclass_items():
    class Dict(dict):
        def items(self):
            return [("override", 2)]

    assert json.dumps(Dict(real=1)) == '{"override": 2}'
    assert json.dumps(Dict()) == "{}"


def main():
    print(
        json.dumps(
            {
                "kind": "provenance",
                "python": sys.version,
                "executable": sys.executable,
                "json_source": json.__file__,
                "native_scanner": json.scanner.make_scanner.__module__,
                "native_encoder": json.encoder.c_make_encoder.__module__,
            }
        )
    )
    failed = 0
    tests = sorted(
        (name, fn) for name, fn in globals().items() if name.startswith("test_")
    )
    for name, test in tests:
        try:
            test()
        except Exception as exc:
            failed += 1
            print(
                json.dumps(
                    {
                        "test": name,
                        "status": "FAIL",
                        "error": type(exc).__name__,
                        "message": str(exc),
                    }
                )
            )
        else:
            print(json.dumps({"test": name, "status": "PASS"}))
    print(
        json.dumps(
            {
                "kind": "summary",
                "tests": len(tests),
                "passed": len(tests) - failed,
                "failed": failed,
            }
        )
    )
    return bool(failed)


if __name__ == "__main__":
    sys.exit(main())
