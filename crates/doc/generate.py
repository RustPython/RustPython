#!/usr/bin/env python
import argparse
import builtins
import inspect
import json
import os
import pathlib
import platform
import re
import sys
import types
import typing
import warnings
from importlib.machinery import EXTENSION_SUFFIXES, ExtensionFileLoader

try:
    from inspect import _getowndoc
except ImportError:
    from pydoc import _getowndoc

if typing.TYPE_CHECKING:
    from collections.abc import Iterable

OUTPUT_FILE = pathlib.Path(__file__).parent / "generated" / f"{sys.platform}.json"
OUTPUT_FILE.parent.mkdir(exist_ok=True)

UNICODE_ESCAPE = re.compile(r"\\u([0-9]+)")

HEAPTYPE = 1 << 9
IMMUTABLETYPE = 1 << 8
C_DESCRIPTORS = (
    types.MethodDescriptorType,
    types.WrapperDescriptorType,
    types.ClassMethodDescriptorType,
)

IGNORED_MODULES = {"this", "antigravity"}
# Each name here is skipped because following it records a bogus path, not
# because the attribute lacks a docstring:
# __annotations__ / __dict__: the mapping already visited via getmembers.
# __class__ / __base__ / __bases__: re-enters the metaclass or a base under
# this object's path.
# __doc__: the docstring text is the parent entry.
# __file__ / __name__ / __qualname__ / __module__: plain metadata strings.
IGNORED_ATTRS = {
    "__annotations__",
    "__base__",
    "__bases__",
    "__class__",
    "__dict__",
    "__doc__",
    "__file__",
    "__module__",
    "__name__",
    "__qualname__",
}


type Parts = tuple[str, ...]


class DocEntry(typing.NamedTuple):
    parts: Parts
    raw_doc: str | None

    @property
    def key(self) -> str:
        return ".".join(self.parts)

    @property
    def doc(self) -> str:
        assert self.raw_doc is not None

        return re.sub(UNICODE_ESCAPE, r"\\u{\1}", self.raw_doc)


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
    if inspect.getmodule(obj) is module:
        return True
    # Some C modules (e.g. _ast) set __module__ to a different name (e.g. "ast"),
    # causing inspect.getmodule() to return a different module object.
    # Fall back to checking the module's namespace directly.
    obj_name = getattr(obj, "__name__", None)
    if obj_name is not None:
        return module.__dict__.get(obj_name) is obj
    # An instance such as sys.flags belongs to the module of its type.
    return type(obj).__module__ == module.__name__


def iter_modules() -> "Iterable[types.ModuleType]":
    """
    Yields
    ------
    :class:`types.Module`
        Python modules.
    """
    for module_name in sorted(sys.stdlib_module_names - IGNORED_MODULES):
        try:
            with warnings.catch_warnings():
                warnings.filterwarnings("ignore", category=DeprecationWarning)
                module = __import__(module_name)
        except ImportError:
            warnings.warn(f"Could not import {module_name}", category=ImportWarning)
            continue

        yield module


def traverse(
    obj: typing.Any, module: types.ModuleType, parts: Parts = ()
) -> "typing.Iterable[DocEntry]":
    if inspect.ismodule(obj):
        parts += (obj.__name__,)

    if any(f(obj) for f in (inspect.ismodule, inspect.isclass, inspect.isbuiltin)):
        yield DocEntry(parts, _getowndoc(obj))

    if inspect.isclass(obj):
        # getmembers() shows the class's own metadata for these names (e.g.
        # coroutine.__name__ is 'coroutine'), hiding the instance descriptor.
        for name in sorted(IGNORED_ATTRS):
            attr = next(
                (vars(base)[name] for base in obj.__mro__ if name in vars(base)),
                None,
            )
            if isinstance(
                attr, (types.GetSetDescriptorType, types.MemberDescriptorType)
            ):
                yield DocEntry(parts + (name,), _getowndoc(attr))

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
        # A native class whose metaclass is not `type` is still a class.
        # Recurse only from a module so a base stored on the class is not walked
        # again under this class's path.
        recurse_class = (
            inspect.ismodule(obj)
            and inspect.isclass(attr)
            and not is_python_class(attr)
        )

        if is_type_or_builtin or recurse_class:
            yield from traverse(attr, module, new_parts)
            declared = (
                getattr(attr, "__module__", None),
                getattr(attr, "__name__", None),
            )
            if recurse_class and declared != (module.__name__, name) and declared[1]:
                key_mod = declared[0] or module.__name__
                yield from traverse(attr, module, (key_mod, declared[1]))
            continue

        is_callable = (
            callable(attr)
            or not issubclass(attr_typ, type)
            or attr_typ.__name__ in ("getset_descriptor", "member_descriptor")
        )

        is_func = any(
            f(attr)
            for f in (inspect.isfunction, inspect.ismethod, inspect.ismethoddescriptor)
        )

        if is_callable or is_func:
            yield DocEntry(new_parts, _getowndoc(attr))
            # A native type the module exposes only through an instance
            # (e.g. the type of sys.flags).
            if (
                inspect.ismodule(obj)
                and not inspect.isclass(attr)
                and attr_typ.__module__ == obj.__name__
                and getattr(obj, attr_typ.__name__, None) is not attr_typ
                and not is_python_class(attr_typ)
            ):
                yield from traverse(attr_typ, module, parts + (attr_typ.__name__,))


def is_python_class(typ: type) -> bool:
    """Whether a class was defined in Python code rather than in C."""
    if not typ.__flags__ & HEAPTYPE or typ.__flags__ & IMMUTABLETYPE:
        return False
    # C types own their slot wrappers and method descriptors even when their
    # __module__ names a Python module (e.g. ast.AST). Python classes may hold
    # descriptors copied from a base (e.g. an enum's str.__format__).
    if any(
        isinstance(attr, C_DESCRIPTORS) and attr.__objclass__ is typ
        for attr in vars(typ).values()
    ):
        return False
    module = sys.modules.get(typ.__module__)
    return module is not None and not is_c_extension(module)


def is_native_exposed(obj: typing.Any) -> bool:
    """A C-implemented object that can carry a stored docstring."""
    if inspect.isclass(obj):
        return not is_python_class(obj)
    return isinstance(
        obj,
        (
            types.BuiltinFunctionType,
            types.BuiltinMethodType,
            *C_DESCRIPTORS,
            types.GetSetDescriptorType,
            types.MemberDescriptorType,
        ),
    )


def iter_native_on_python_modules(
    modules: "Iterable[types.ModuleType]",
) -> "Iterable[DocEntry]":
    """Native objects exposed by pure-Python modules.

    Each is keyed by its own ``__module__`` and ``__name__`` (``ssl.SSLError``
    comes from ``_ssl``). Builtin functions without a ``__module__``, such as
    the codec error handlers, are keyed under the module exposing them.
    """
    for module in modules:
        if is_c_extension(module):
            continue
        for name, attr in inspect.getmembers(module):
            if name in IGNORED_ATTRS or not is_native_exposed(attr):
                continue
            # A method bound to some object (e.g. `frozenset(...).__contains__`)
            # is documented on its type.
            owner = getattr(attr, "__self__", None)
            if owner is not None and not inspect.ismodule(owner):
                continue
            parts = (
                getattr(attr, "__module__", None) or module.__name__,
                getattr(attr, "__name__", None) or name,
            )
            if inspect.isclass(attr):
                yield from traverse(attr, module, parts)
            else:
                yield DocEntry(parts, _getowndoc(attr))


def find_doc_entries() -> "Iterable[DocEntry]":
    # Import every module first so is_python_class() sees the same
    # sys.modules regardless of import order.
    modules = list(iter_modules())
    for module in filter(is_c_extension, modules):
        yield from traverse(module, module)
    yield from iter_native_on_python_modules(modules)
    yield from (doc_entry for doc_entry in traverse(__builtins__, __builtins__))

    builtin_types = object.__subclasses__()

    # Add types from the types module (e.g., ModuleType, FunctionType, etc.)
    for name in dir(types):
        if name.startswith("_"):
            continue
        obj = getattr(types, name)
        if isinstance(obj, type):
            builtin_types.append(obj)

    for typ in builtin_types:
        # Classes defined in Python get their docs from source.
        if is_python_class(typ):
            continue
        module_names = ["builtins"]
        # Also key a type by its own module, since a type found only through
        # object.__subclasses__() (e.g. _ctypes._CData) is looked up there.
        # Skip it when that name in the module is another object.
        module = sys.modules.get(typ.__module__)
        if (
            module is not None
            and module is not builtins
            and getattr(module, typ.__name__, typ) is typ
        ):
            module_names.append(typ.__module__)
        for module_name in module_names:
            parts = (module_name, typ.__name__)
            yield DocEntry(parts, _getowndoc(typ))
            yield from traverse(typ, __builtins__, parts)


def main():
    docs = {
        entry.key: entry.doc
        for entry in find_doc_entries()
        if entry.raw_doc is not None and isinstance(entry.raw_doc, str)
    }
    dumped = json.dumps(docs, sort_keys=True, indent=4)
    OUTPUT_FILE.write_text(dumped)


if __name__ == "__main__":
    main()
