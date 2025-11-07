#!/usr/bin/env python
import inspect
import json
import os
import pathlib
import platform
import pydoc
import re
import sys
import textwrap
import types
import typing
import warnings
from importlib.machinery import EXTENSION_SUFFIXES, ExtensionFileLoader

if typing.TYPE_CHECKING:
    from collections.abc import Iterable

UNICODE_ESCAPE = re.compile(r"\\u([0-9]+)")

IGNORED_MODULES = {"this", "antigravity"}
IGNORED_ATTRS = {
    "__annotations__",
    "__class__",
    "__dict__",
    "__dir__",
    "__doc__",
    "__file__",
    "__name__",
    "__qualname__",
}

CRATE_DIR = pathlib.Path(__file__).parent
OUT_FILE = CRATE_DIR / "src" / f"{sys.platform}.inc.rs"

type Parts = tuple[str, ...]


class DocEntry(typing.NamedTuple):
    parts: Parts
    doc: str | None

    @property
    def phf_entry(self) -> str:
        escaped = re.sub(UNICODE_ESCAPE, r"\\u{\1}", inspect.cleandoc(self.doc))
        doc = json.dumps(escaped)

        key = json.dumps(".".join(self.parts))
        return f"{key} => {doc}"


def is_c_extension(module: types.ModuleType) -> bool:
    """
    Check whether a module was written in C.

    Returns
    -------
    bool

    Notes
    -----
    Adapted from: https://stackoverflow.com/a/39304199
    """
    loader = getattr(module, "__loader__", None)
    if isinstance(loader, ExtensionFileLoader):
        return True

    try:
        inspect.getsource(module)
    except (OSError, TypeError):
        return True

    try:
        module_filename = inspect.getfile(module)
    except TypeError:
        return True

    module_filetype = os.path.splitext(module_filename)[1]
    return module_filetype in EXTENSION_SUFFIXES


def is_child_of(obj: typing.Any, module: types.ModuleType) -> bool:
    """
    Whether or not an object is a child of a module.

    Returns
    -------
    bool
    """
    return inspect.getmodule(obj) is module


def iter_modules() -> "Iterable[types.ModuleType]":
    """
    Yields
    ------
    :class:`types.Module`
        Python modules.
    """
    for module_name in sys.stdlib_module_names - IGNORED_MODULES:
        try:
            with warnings.catch_warnings():
                warnings.filterwarnings("ignore", category=DeprecationWarning)
                module = __import__(module_name)
        except ImportError:
            warnings.warn(f"Could not import {module_name}", category=ImportWarning)
            continue

        yield module


def iter_c_modules() -> "Iterable[types.ModuleType]":
    """
    Yields
    ------
    :class:`types.Module`
        Modules that are written in C. (not pure python)
    """
    yield from filter(is_c_extension, iter_modules())


def traverse(
    obj: typing.Any, module: types.ModuleType, parts: Parts = ()
) -> "typing.Iterable[DocEntry]":
    if inspect.ismodule(obj):
        parts += (obj.__name__,)

    if any(f(obj) for f in (inspect.ismodule, inspect.isclass, inspect.isbuiltin)):
        yield DocEntry(parts, pydoc._getowndoc(obj))

    for name, attr in inspect.getmembers(obj):
        if name in IGNORED_ATTRS:
            continue

        if attr == obj:
            continue

        if (module is obj) and (not is_child_of(attr, module)):
            continue

        # Don't recurse into modules imported by our module. i.e. `ipaddress.py` imports `re` don't traverse `re`
        if (not inspect.ismodule(obj)) and inspect.ismodule(attr):
            continue

        new_parts = parts + (name,)

        attr_typ = type(attr)
        is_type_or_builtin = any(attr_typ is x for x in (type, type(__builtins__)))

        if is_type_or_builtin:
            yield from traverse(attr, module, new_parts)
            continue

        is_callable = (
            callable(attr)
            or not issubclass(attr_typ, type)
            or attr_typ.__name__ in ("getset_descriptor", "member_descriptor")
        )

        is_func = any(
            f(obj)
            for f in (inspect.isfunction, inspect.ismethod, inspect.ismethoddescriptor)
        )

        if is_callable or is_func:
            yield DocEntry(new_parts, pydoc._getowndoc(attr))


def find_doc_entires() -> "Iterable[DocEntry]":
    yield from (
        doc_entry
        for module in iter_c_modules()
        for doc_entry in traverse(module, module)
    )
    yield from (doc_entry for doc_entry in traverse(__builtins__, __builtins__))

    builtin_types = [
        type(None),
        type(bytearray().__iter__()),
        type(bytes().__iter__()),
        type(dict().__iter__()),
        type(dict().items()),
        type(dict().items().__iter__()),
        type(dict().values()),
        type(dict().values().__iter__()),
        type(lambda: ...),
        type(list().__iter__()),
        type(memoryview(b"").__iter__()),
        type(range(0).__iter__()),
        type(set().__iter__()),
        type(str().__iter__()),
        type(tuple().__iter__()),
    ]
    for typ in builtin_types:
        parts = ("builtins", typ.__name__)
        yield DocEntry(parts, pydoc._getowndoc(typ))
        yield from traverse(typ, __builtins__, parts)


def main():
    doc_entries = {
        doc_entry.phf_entry
        for doc_entry in find_doc_entires()
        if doc_entry.doc is not None
    }

    lines = ",\n".join(sorted(doc_entries))
    lines = textwrap.indent(lines, prefix=" " * 4)

    python_version = platform.python_version()
    script_name = pathlib.Path(__file__).name

    out = f"""
// This file was auto generated by: {script_name}
// CPython version: {python_version}
// spell-checker: disable

use phf::{{Map, phf_map}};

pub static DB: Map<&'static str, &'static str> = phf_map! {{
{lines}
}};
""".lstrip()

    OUT_FILE.write_text(out)


if __name__ == "__main__":
    main()
