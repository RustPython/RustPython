"""Marshal retains filename updates made while importing relocated bytecode."""

import _imp
import io
import marshal
import types


def nested_codes(code):
    yield code
    for constant in code.co_consts:
        if isinstance(constant, types.CodeType):
            yield from nested_codes(constant)


for version in range(marshal.version + 1):
    code = compile("def outer():\n    return lambda: 42\n", "original.py", "exec")
    for filename in ("moved.py", "moved-again.py"):
        _imp._fix_co_filename(code, filename)
        restored = marshal.loads(marshal.dumps(code, version))
        assert [item.co_filename for item in nested_codes(restored)] == [filename] * 3

    foreign = compile("pass", "foreign-original.py", "exec")
    _imp._fix_co_filename(foreign, "foreign-moved.py")
    combined = code.replace(co_consts=code.co_consts + (foreign,))
    _imp._fix_co_filename(combined, "module-moved.py")
    stream = io.BytesIO()
    marshal.dump(combined, stream, version)
    stream.seek(0)
    restored = marshal.load(stream)
    assert restored.co_filename == "module-moved.py"
    assert restored.co_consts[-1].co_filename == "foreign-moved.py"
    assert foreign.co_filename == "foreign-moved.py"
