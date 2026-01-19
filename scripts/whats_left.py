#!/usr/bin/env -S python3 -I
# /// script
# requires-python = ">=3.14"
# ///

# This script generates Lib/snippets/whats_left_data.py with these variables defined:
# expected_methods - a dictionary mapping builtin objects to their methods
# cpymods - a dictionary mapping module names to their contents
# libdir - the location of RustPython's Lib/ directory.

#
# TODO: include this:
# which finds all modules it has available and
# creates a Python dictionary mapping module names to their contents, which is
# in turn used to generate a second Python script that also finds which modules
# it has available and compares that against the first dictionary we generated.
# We then run this second generated script with RustPython.

import argparse
import inspect
import json
import os
import platform
import re
import subprocess
import sys
import warnings
from pydoc import ModuleScanner

if not sys.flags.isolated:
    print("running without -I option.")
    print("python -I scripts/whats_left.py")
    exit(1)

GENERATED_FILE = "extra_tests/not_impl.py"

implementation = platform.python_implementation()
if implementation != "CPython":
    sys.exit(f"whats_left.py must be run under CPython, got {implementation} instead")
if sys.version_info[:2] < (3, 14):
    sys.exit(
        f"whats_left.py must be run under CPython 3.14 or newer, got {implementation} {sys.version} instead. If you have uv, try `uv run python -I scripts/whats_left.py` to select a proper Python interpreter easier."
    )


def parse_args():
    parser = argparse.ArgumentParser(description="Process some integers.")
    parser.add_argument(
        "--signature",
        action="store_true",
        help="print functions whose signatures don't match CPython's",
    )
    parser.add_argument(
        "--doc",
        action="store_true",
        help="print elements whose __doc__ don't match CPython's",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="print output as JSON (instead of line by line)",
    )
    parser.add_argument(
        "--features",
        action="store",
        help="which features to enable when building RustPython (default: ssl)",
        default="ssl",
    )

    args = parser.parse_args()
    return args


args = parse_args()

# CPython specific modules (mostly consisting of templates/tests)
CPYTHON_SPECIFIC_MODS = {
    "xxmodule",
    "xxsubtype",
    "xxlimited",
    "_xxtestfuzz",
    "_testbuffer",
    "_testcapi",
    "_testimportmultiple",
    "_testinternalcapi",
    "_testmultiphase",
    "_testlimitedcapi",
}

IGNORED_MODULES = {"this", "antigravity"} | CPYTHON_SPECIFIC_MODS

sys.path = [
    path
    for path in sys.path
    if ("site-packages" not in path and "dist-packages" not in path)
]


def attr_is_not_inherited(type_, attr):
    """
    returns True if type_'s attr is not inherited from any of its base classes
    """
    bases = type_.__mro__[1:]
    return getattr(type_, attr) not in (getattr(base, attr, None) for base in bases)


def extra_info(obj):
    if callable(obj) and not inspect._signature_is_builtin(obj):
        doc = inspect.getdoc(obj)
        try:
            sig = str(inspect.signature(obj))
            # remove function memory addresses
            return {
                "sig": re.sub(" at 0x[0-9A-Fa-f]+", " at 0xdeadbeef", sig),
                "doc": doc,
            }
        except Exception as e:
            exception = repr(e)
            # CPython uses ' RustPython uses "
            if exception.replace('"', "'").startswith("ValueError('no signature found"):
                return {
                    "sig": "ValueError('no signature found')",
                    "doc": doc,
                }

            return {
                "sig": exception,
                "doc": doc,
            }

    return {
        "sig": None,
        "doc": None,
    }


def name_sort_key(name):
    if name == "builtins":
        return ""
    if name[0] == "_":
        return name[1:] + "1"
    return name + "2"


def gen_methods():
    types = [
        bool,
        bytearray,
        bytes,
        complex,
        dict,
        enumerate,
        filter,
        float,
        frozenset,
        int,
        list,
        map,
        memoryview,
        range,
        set,
        slice,
        str,
        super,
        tuple,
        object,
        zip,
        classmethod,
        staticmethod,
        property,
        Exception,
        BaseException,
    ]
    objects = [t.__name__ for t in types]
    objects.append("type(None)")

    iters = [
        "type(bytearray().__iter__())",
        "type(bytes().__iter__())",
        "type(dict().__iter__())",
        "type(dict().values().__iter__())",
        "type(dict().items().__iter__())",
        "type(dict().values())",
        "type(dict().items())",
        "type(set().__iter__())",
        "type(list().__iter__())",
        "type(range(0).__iter__())",
        "type(str().__iter__())",
        "type(tuple().__iter__())",
        "type(memoryview(bytearray(b'0')).__iter__())",
    ]

    methods = {}
    for typ_code in objects + iters:
        typ = eval(typ_code)
        attrs = []
        for attr in dir(typ):
            # Skip attributes in dir() but not actually accessible (e.g., descriptor that raises)
            if not hasattr(typ, attr):
                continue
            if attr_is_not_inherited(typ, attr):
                attrs.append((attr, extra_info(getattr(typ, attr))))
        methods[typ.__name__] = (typ_code, extra_info(typ), attrs)

    output = "expected_methods = {\n"
    for name in sorted(methods.keys(), key=name_sort_key):
        typ_code, extra, attrs = methods[name]
        output += f" '{name}': ({typ_code}, {extra!r}, [\n"
        for attr, attr_extra in attrs:
            output += f"    ({attr!r}, {attr_extra!r}),\n"
        output += " ]),\n"
        if typ_code != objects[-1]:
            output += "\n"
    output += "}\n\n"
    return output


def scan_modules():
    """taken from the source code of help('modules')

    https://github.com/python/cpython/blob/63298930fb531ba2bb4f23bc3b915dbf1e17e9e1/Lib/pydoc.py#L2178"""
    modules = {}

    def callback(path, modname, desc, modules=modules):
        if modname and modname[-9:] == ".__init__":
            modname = modname[:-9] + " (package)"
        if modname.find(".") < 0:
            modules[modname] = 1

    def onerror(modname):
        callback(None, modname, None)

    with warnings.catch_warnings():
        # ignore warnings from importing deprecated modules
        warnings.simplefilter("ignore")
        ModuleScanner().run(callback, onerror=onerror)
    return list(modules.keys())


def import_module(module_name):
    import io
    from contextlib import redirect_stdout

    # Importing modules causes ('Constant String', 2, None, 4) and
    # "Hello world!" to be printed to stdout.
    f = io.StringIO()
    with warnings.catch_warnings(), redirect_stdout(f):
        # ignore warnings caused by importing deprecated modules
        warnings.filterwarnings("ignore", category=DeprecationWarning)
        try:
            module = __import__(module_name)
        except Exception as e:
            return e
    return module


def is_child(module, item):
    import inspect

    item_mod = inspect.getmodule(item)
    return item_mod is module


def dir_of_mod_or_error(module_name, keep_other=True):
    module = import_module(module_name)
    item_names = sorted(set(dir(module)))
    result = {}
    for item_name in item_names:
        item = getattr(module, item_name)
        # don't repeat items imported from other modules
        if keep_other or is_child(module, item) or inspect.getmodule(item) is None:
            result[item_name] = extra_info(item)
    return result


def gen_modules():
    # check name because modules listed have side effects on import,
    # e.g. printing something or opening a webpage
    modules = {}
    for mod_name in sorted(scan_modules(), key=name_sort_key):
        if mod_name in IGNORED_MODULES:
            continue
        # when generating CPython list, ignore items defined by other modules
        dir_result = dir_of_mod_or_error(mod_name, keep_other=False)
        if isinstance(dir_result, Exception):
            print(
                f"!!! {mod_name} skipped because {type(dir_result).__name__}: {str(dir_result)}",
                file=sys.stderr,
            )
            continue
        modules[mod_name] = dir_result
    return modules


output = """\
# WARNING: THIS IS AN AUTOMATICALLY GENERATED FILE
# EDIT extra_tests/not_impl_gen.sh, NOT THIS FILE.
# RESULTS OF THIS TEST DEPEND ON THE CPYTHON
# VERSION AND PYTHON ENVIRONMENT USED
# TO RUN not_impl_mods_gen.py

"""

output += gen_methods()
output += f"""
cpymods = {gen_modules()!r}
libdir = {os.path.abspath("Lib/").encode("utf8")!r}

"""

# Copy the source code of functions we will reuse in the generated script
REUSED = [
    attr_is_not_inherited,
    extra_info,
    dir_of_mod_or_error,
    import_module,
    is_child,
]
for fn in REUSED:
    output += "".join(inspect.getsourcelines(fn)[0]) + "\n\n"

# Prevent missing variable linter errors from compare()
expected_methods = {}
cpymods = {}
libdir = ""


# This function holds the source code that will be run under RustPython
def compare():
    import inspect
    import io
    import json
    import os
    import platform
    import re
    import sys
    import warnings
    from contextlib import redirect_stdout

    def method_incompatibility_reason(typ, method_name, real_method_value):
        has_method = hasattr(typ, method_name)
        if not has_method:
            return ""

        is_inherited = not attr_is_not_inherited(typ, method_name)
        if is_inherited:
            return "(inherited)"

        value = extra_info(getattr(typ, method_name))
        if value != real_method_value:
            return f"{value} != {real_method_value}"

        return None

    not_implementeds = {}
    for name, (typ, real_value, methods) in expected_methods.items():
        missing_methods = {}
        for method, real_method_value in methods:
            reason = method_incompatibility_reason(typ, method, real_method_value)
            if reason is not None:
                missing_methods[method] = reason
        if missing_methods:
            not_implementeds[name] = missing_methods

    if platform.python_implementation() == "CPython":
        if not_implementeds:
            sys.exit(
                f"ERROR: CPython should have all the methods but missing: {not_implementeds}"
            )

    mod_names = [
        name.decode()
        for name, ext in map(os.path.splitext, os.listdir(libdir))
        if ext == b".py" or os.path.isdir(os.path.join(libdir, name))
    ]
    mod_names += list(sys.builtin_module_names)
    # Remove easter egg modules
    mod_names = sorted(set(mod_names) - {"this", "antigravity"})

    rustpymods = {mod: dir_of_mod_or_error(mod) for mod in mod_names}

    result = {
        "cpython_modules": {},
        "implemented": {},
        "not_implemented": {},
        "failed_to_import": {},
        "missing_items": {},
        "mismatched_items": {},
        "mismatched_doc_items": {},
    }
    for modname, cpymod in cpymods.items():
        rustpymod = rustpymods.get(modname)
        if rustpymod is None:
            result["not_implemented"][modname] = None
        elif isinstance(rustpymod, Exception):
            result["failed_to_import"][modname] = rustpymod.__class__.__name__ + str(
                rustpymod
            )
        else:
            implemented_items = sorted(set(cpymod) & set(rustpymod))
            mod_missing_items = set(cpymod) - set(rustpymod)
            mod_missing_items = sorted(
                f"{modname}.{item}" for item in mod_missing_items
            )
            mod_mismatched_items = [
                (f"{modname}.{item}", rustpymod[item]["sig"], cpymod[item]["sig"])
                for item in implemented_items
                if rustpymod[item]["sig"] != cpymod[item]["sig"]
                and not isinstance(cpymod[item]["sig"], Exception)
            ]
            mod_mismatched_doc_items = [
                (f"{modname}.{item}", rustpymod[item]["doc"], cpymod[item]["doc"])
                for item in implemented_items
                if rustpymod[item]["doc"] != cpymod[item]["doc"]
            ]
            if mod_missing_items or mod_mismatched_items:
                if mod_missing_items:
                    result["missing_items"][modname] = mod_missing_items
                if mod_mismatched_items:
                    result["mismatched_items"][modname] = mod_mismatched_items
                if mod_mismatched_doc_items:
                    result["mismatched_doc_items"][modname] = mod_mismatched_doc_items
            else:
                result["implemented"][modname] = None

    result["cpython_modules"] = cpymods
    result["not_implementeds"] = not_implementeds

    print(json.dumps(result))


def remove_one_indent(s):
    indent = "    "
    return s[len(indent) :] if s.startswith(indent) else s


compare_src = inspect.getsourcelines(compare)[0][1:]
output += "".join(remove_one_indent(line) for line in compare_src)

with open(GENERATED_FILE, "w", encoding="utf-8") as f:
    f.write(output + "\n")


subprocess.run(
    ["cargo", "build", "--release", f"--features={args.features}"], check=True
)
result = subprocess.run(
    [
        "cargo",
        "run",
        "--release",
        f"--features={args.features}",
        "-q",
        "--",
        GENERATED_FILE,
    ],
    env={**os.environ.copy(), "RUSTPYTHONPATH": "Lib"},
    text=True,
    capture_output=True,
)
# The last line should be json output, the rest of the lines can contain noise
# because importing certain modules can print stuff to stdout/stderr
print(result.stderr, file=sys.stderr)
result = json.loads(result.stdout.splitlines()[-1])

if args.json:
    print(json.dumps(result))
    sys.exit()


# missing entire modules
print("# modules")
for modname in result["not_implemented"]:
    print(modname, "(entire module)")
for modname, exception in result["failed_to_import"].items():
    print(f"{modname} (exists but not importable: {exception})")

# missing from builtins
print("\n# builtin items")
for module, missing_methods in result["not_implementeds"].items():
    for method, reason in missing_methods.items():
        print(f"{module}.{method}" + (f" {reason}" if reason else ""))

# missing from modules
print("\n# stdlib items")
for modname, missing in result["missing_items"].items():
    for item in missing:
        print(item)

if args.signature:
    print("\n# mismatching signatures (warnings)")
    for modname, mismatched in result["mismatched_items"].items():
        for i, (item, rustpy_value, cpython_value) in enumerate(mismatched):
            if cpython_value and cpython_value.startswith("ValueError("):
                continue  # these items will never match
            if rustpy_value is None or rustpy_value.startswith("ValueError("):
                rustpy_value = f" {rustpy_value}"
            print(f"{item}{rustpy_value}")
            if cpython_value is None:
                cpython_value = f" {cpython_value}"
            print(f"{' ' * len(item)}{cpython_value}")
            if i < len(mismatched) - 1:
                print()

if args.doc:
    print("\n# mismatching `__doc__`s (warnings)")
    for modname, mismatched in result["mismatched_doc_items"].items():
        for item, rustpy_doc, cpython_doc in mismatched:
            print(f"{item} {repr(rustpy_doc)} != {repr(cpython_doc)}")


print()
print("# summary")
for error_type, modules in result.items():
    print("# ", error_type, len(modules))
