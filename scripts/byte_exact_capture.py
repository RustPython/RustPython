"""Compile one ordinary request; emit unmodified marshal before field inspection."""

import json
import marshal
import struct
import sys
import types

FIELDS = (
    "co_argcount",
    "co_posonlyargcount",
    "co_kwonlyargcount",
    "co_nlocals",
    "co_stacksize",
    "co_flags",
    "co_code",
    "co_consts",
    "co_names",
    "co_varnames",
    "co_freevars",
    "co_cellvars",
    "co_filename",
    "co_name",
    "co_qualname",
    "co_firstlineno",
    "co_linetable",
    "co_exceptiontable",
)


def describe(value):
    kind = type(value)
    if kind is types.CodeType:
        return {
            "type": "code",
            "fields": {name: describe(getattr(value, name)) for name in FIELDS},
        }
    if value is None or value is Ellipsis:
        return {"type": kind.__name__}
    if kind in (bool, int, str):
        return {"type": kind.__name__, "value": value}
    if kind is bytes:
        return {"type": "bytes", "value": list(value)}
    if kind is float:
        return {"type": "float", "bits": list(struct.pack("<d", value))}
    if kind is complex:
        return {
            "type": "complex",
            "bits": list(struct.pack("<dd", value.real, value.imag)),
        }
    if kind in (tuple, frozenset):
        # Retain observed iteration order. No diagnostic or wire normalization.
        return {"type": kind.__name__, "items": [describe(item) for item in value]}
    raise TypeError("uncaptured constant type: " + kind.__name__)


def capture(request):
    code = compile(
        request["source"],
        request["filename"],
        request["mode"],
        flags=request["flags"],
        dont_inherit=True,
        optimize=request["optimize"],
    )
    raw = list(marshal.dumps(code, 6))
    # Read exported fields only after serialization: getters may acquire owners.
    fields = describe(code)
    import _frozen_importlib_external

    return {
        "marshal": raw,
        "code": fields,
        "magic": list(_frozen_importlib_external.MAGIC_NUMBER),
        "marshal_version": marshal.version,
        "version": list(sys.version_info),
        "runtime": runtime_provenance(),
    }


def runtime_provenance():
    modules = {}
    for name in ("json", "types", "struct", "marshal", "_frozen_importlib_external"):
        module = sys.modules[name]
        spec = getattr(module, "__spec__", None)
        modules[name] = {
            "file": getattr(module, "__file__", None),
            "origin": getattr(spec, "origin", None),
            "cached": getattr(spec, "cached", None),
        }
    return {
        "path": list(sys.path),
        "pycache_prefix": getattr(sys, "pycache_prefix", None),
        "has_pycache_prefix": hasattr(sys, "pycache_prefix"),
        "dont_write_bytecode": sys.dont_write_bytecode,
        "modules": modules,
    }


if __name__ == "__main__":
    print(json.dumps(capture(json.load(sys.stdin)), sort_keys=True))
