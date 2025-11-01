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


class DocEntry(typing.NamedTuple):
    parts: tuple[str, ...]
    doc: str | None

    @property
    def phf_entry(self) -> str:
        if self.doc:
            escaped = re.sub(UNICODE_ESCAPE, r"\\u{\1}", self.doc)
            dumped = json.dumps(escaped)
            doc = f"Some({dumped})"
        else:
            doc = "None"

        key = json.dumps(".".join(self.parts))
        return f"{key} => {doc}"


def is_c_extension(module: types.ModuleType) -> bool:
    """
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


def is_child(obj: typing.Any, module: types.ModuleType) -> bool:
    return inspect.getmodule(obj) is module


def iter_modules() -> "Iterable[types.ModuleType]":
    """
    Yields
    ------
    :class:`types.Module`
        Modules that are written in C. (not pure python)
    """
    for module_name in sys.stdlib_module_names - IGNORED_MODULES:
        try:
            with warnings.catch_warnings():
                warnings.filterwarnings("ignore", category=DeprecationWarning)
                module = __import__(module_name)
        except ImportError:
            warnings.warn(f"Could not import {module_name}", category=ImportWarning)
            continue

        if not is_c_extension(module):
            continue

        yield module


def traverse(
    obj: typing.Any, module: types.ModuleType, name_parts: tuple[str, ...] = ()
) -> "typing.Iterable[DocEntry]":
    if inspect.ismodule(obj):
        name_parts += (obj.__name__,)

    if any(f(obj) for f in (inspect.ismodule, inspect.isclass, inspect.isbuiltin)):
        yield DocEntry(name_parts, pydoc._getdoc(obj))

    for name, attr in inspect.getmembers(obj):
        if name in IGNORED_ATTRS:
            continue

        if attr == obj:
            continue

        parts = name_parts + (name,)

        if (module is obj) and (not is_child(attr, module)):
            continue

        if (not inspect.ismodule(obj)) and inspect.ismodule(attr):
            continue

        new_name_parts = name_parts + (name,)

        attr_typ = type(attr)
        is_type_or_module = (attr_typ is type) or (attr_typ is type(__builtins__))

        if is_type_or_module:
            yield from traverse(attr, module, new_name_parts)
            continue

        if (
            callable(attr)
            or not issubclass(attr_typ, type)
            or attr_typ.__name__ in ("getset_descriptor", "member_descriptor")
        ) or any(
            f(obj)
            for f in (
                inspect.isfunction,
                inspect.ismethod,
                inspect.ismethoddescriptor,
            )
        ):
            yield DocEntry(new_name_parts, pydoc._getdoc(attr))


def find_doc_entires() -> "Iterable[DocEntry]":
    yield from (
        doc_entry for module in iter_modules() for doc_entry in traverse(module, module)
    )
    yield from (doc_entry for doc_entry in traverse(__builtins__, __builtins__))

    builtin_types = [
        type(bytearray().__iter__()),
        type(bytes().__iter__()),
        type(dict().__iter__()),
        type(dict().values().__iter__()),
        type(dict().items().__iter__()),
        type(dict().values()),
        type(dict().items()),
        type(set().__iter__()),
        type(list().__iter__()),
        type(range(0).__iter__()),
        type(str().__iter__()),
        type(tuple().__iter__()),
        type(None),
        type(lambda: ...),
    ]
    for typ in builtin_types:
        name_parts = ("builtins", typ.__name__)
        yield DocEntry(name_parts, pydoc._getdoc(typ))
        yield from traverse(typ, __builtins__, name_parts)


def main():
    doc_entries = {doc_entry.phf_entry for doc_entry in find_doc_entires()}

    lines = ",\n".join(sorted(doc_entries))
    lines = textwrap.indent(lines, prefix=" " * 4)

    python_version = platform.python_version()
    script_name = pathlib.Path(__file__).name

    out = f"""
// This file was auto generated by: {script_name}
// CPython version: {python_version}
phf::phf_map! {{
{lines}
}}
""".lstrip()

    OUT_FILE.write_text(out)


if __name__ == "__main__":
    main()
