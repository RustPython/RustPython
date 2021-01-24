# It's recommended to run this with `python3 -I not_impl_gen.py`, to make sure
# that nothing in your global Python environment interferes with what's being
# extracted here.
#
# This script generates Lib/snippets/whats_left_data.py with these variables defined:
# expected_methods - a dictionary mapping builtin objects to their methods
# cpymods - a dictionary mapping module names to their contents
# libdir - the location of RustPython's Lib/ directory.

import re
import os
import sys
import warnings
import inspect
from pydoc import ModuleScanner


sys.path = list(
    filter(
        lambda path: "site-packages" not in path and "dist-packages" not in path,
        sys.path,
    )
)


def attr_is_not_inherited(type_, attr):
    """
    returns True if type_'s attr is not inherited from any of its base classes
    """
    bases = type_.__mro__[1:]
    return getattr(type_, attr) not in (getattr(base, attr, None) for base in bases)


# TODO: move this function to a shared library both CPython and RustPython import
def extra_info(obj):
    if callable(obj):
        # TODO: check for the correct thing above and remove try
        try:
            sig = str(inspect.signature(obj))
            # remove function memory addresses
            return re.sub(" at 0x[0-9A-Fa-f]+", " at 0xdeadbeef", sig)
        except Exception:
            return None
    return None


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
    ]

    methods = {}
    for typ_code in objects + iters:
        typ = eval(typ_code)
        methods[typ.__name__] = (
            typ_code,
            extra_info(typ),
            [(attr, extra_info(attr)) for attr in dir(typ) if attr_is_not_inherited(typ, attr)],
        )

    output = "expected_methods = {\n"
    for name, (typ_code, extra, attrs) in methods.items():
        output += f" '{name}': ({typ_code}, {extra!r}, [\n"
        for attr, attr_extra in attrs:
            output += f"    ({attr!r}, {attr_extra!r}),\n"
        output += " ]),\n"
        # TODO: why?
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
        warnings.simplefilter("ignore")  # ignore warnings from importing deprecated modules
        ModuleScanner().run(callback, onerror=onerror)
    return list(modules.keys())


# TODO: move this function to a shared library both CPython and RustPython import
def dir_of_mod_or_error(module_name):
    with warnings.catch_warnings():
        # ignore warnings caused by importing deprecated modules
        warnings.filterwarnings("ignore", category=DeprecationWarning)
        try:
            module = __import__(module_name)
        except Exception as e:
            return e
    item_names = sorted(set(dir(module)))
    result = {}
    for item_name in item_names:
        item = getattr(module, item_name)
        result[item_name] = extra_info(item)
    return result


def gen_modules():
    # check name because modules listed have side effects on import,
    # e.g. printing something or opening a webpage
    modules = {}
    for mod_name in scan_modules():
        if mod_name in ("this", "antigravity"):
            continue
        dir_result = dir_of_mod_or_error(mod_name)
        if isinstance(dir_result, Exception):
            print(
                f"!!! {mod_name} skipped because {type(dir_result).__name__}: {str(dir_result)}",
                file=sys.stderr,
            )
            continue
        modules[mod_name] = dir_result
    return f"""
cpymods = {modules!r}
libdir = {os.path.abspath("../Lib/").encode('utf8')!r}

"""


output = """\
# WARNING: THIS IS AN AUTOMATICALLY GENERATED FILE
# EDIT extra_tests/not_impl_gen.sh, NOT THIS FILE.
# RESULTS OF THIS TEST DEPEND ON THE CPYTHON
# VERSION AND PYTHON ENVIRONMENT USED
# TO RUN not_impl_mods_gen.py

"""

output += gen_methods()
output += gen_modules()

# Prevent missing variable linter errors from compare()
expected_methods = {}
cpymods = {}
libdir = ""
# This function holds the source code that will be run under RustPython
def compare():
    import re
    import os
    import sys
    import warnings
    import inspect
    import platform

    def attr_is_not_inherited(type_, attr):
        """
        returns True if type_'s attr is not inherited from any of its base classes
        """
        bases = type_.__mro__[1:]
        return getattr(type_, attr) not in (getattr(base, attr, None) for base in bases)

    # TODO: move this function to a shared library both CPython and RustPython import
    def extra_info(obj):
        if callable(obj):
            # TODO: check for the correct thing above and remove try
            try:
                sig = str(inspect.signature(obj))
                # remove function memory addresses
                return re.sub(" at 0x[0-9A-Fa-f]+", " at 0xdeadbeef", sig)
            except Exception:
                return None
        return None

    not_implementeds = []
    for name, (typ, real_value, methods) in expected_methods.items():
        for method, method_extra in methods:
            has_method = hasattr(typ, method)
            is_inherited = has_method and not attr_is_not_inherited(typ, method)
            value = extra_info(method)
            if has_method and not is_inherited and value == real_value:
                continue

            if not has_method:
                reason = ""
            elif is_inherited:
                reason = "inherited"
            else:
                reason = f"{value} != {real_value}"
            not_implementeds.append((name, method, reason))

    for module, method, reason in not_implementeds:
        print(f"{module}.{method}" + (f" {reason}" if reason else ""))
    if not not_implementeds:
        print("Not much \\o/")

    if platform.python_implementation() == "CPython":
        if not_implementeds:
            sys.exit("ERROR: CPython should have all the methods")

    # TODO: move this function to a shared library both CPython and RustPython import
    def dir_of_mod_or_error(module_name):
        with warnings.catch_warnings():
            # ignore warnings caused by importing deprecated modules
            warnings.filterwarnings("ignore", category=DeprecationWarning)
            try:
                module = __import__(module_name)
            except Exception as e:
                return e
        item_names = sorted(set(dir(module)))
        result = {}
        for item_name in item_names:
            item = getattr(module, item_name)
            result[item_name] = extra_info(item)
        return result

    mod_names = [
        name.decode()
        for name, ext in map(os.path.splitext, os.listdir(libdir))
        if ext == b".py" or os.path.isdir(os.path.join(libdir, name))
    ]
    mod_names += list(sys.builtin_module_names)
    # Remove easter egg modules
    mod_names = sorted(set(mod_names) - {"this", "antigravity"})

    rustpymods = {mod: dir_of_mod_or_error(mod) for mod in mod_names}

    for modname, cpymod in cpymods.items():
        rustpymod = rustpymods.get(modname, {})
        if isinstance(rustpymod, ImportError):
            print(modname, "(entire module)")
        elif isinstance(rustpymod, Exception):
            print(f"{modname} (exists but not importable: {rustpymod})")
        else:
            implemented_items = sorted(set(cpymod) & set(rustpymod))
            missing_items = set(cpymod) - set(rustpymod)
            if not rustpymod and cpymod:
                print(modname, "(entire module)")  # TODO: why do I have this twice
            else:
                for item in missing_items:
                    print(f"{modname}.{item}")
                for item in implemented_items:
                    if rustpymod[item] != cpymod[item] and cpymod[item] != "None":
                        print(f"{modname}.{item}: {rustpymod[item]} != {cpymod[item]}")


def remove_one_indent(s):
    indent = "    "
    return s[len(indent) :] if s.startswith(indent) else s


compare_src = inspect.getsourcelines(compare)[0][1:]
output += "".join(remove_one_indent(line) for line in compare_src)

with open("snippets/not_impl.py", "w") as f:
    f.write(output + "\n")
