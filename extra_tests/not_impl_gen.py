# It's recommended to run this with `python3 -I not_impl_gen.py`, to make sure
# that nothing in your global Python environment interferes with what's being
# extracted here.

import pkgutil
import os
import sys
import warnings
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


def gen_methods(header, footer, output):
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
    objects = [(t.__name__, t.__name__) for t in types]
    objects.extend([
        ('NoneType', 'type(None)'),
    ])

    iters = [
        ('bytearray_iterator', 'type(bytearray().__iter__())'),
        ('bytes_iterator', 'type(bytes().__iter__())'),
        ('dict_keyiterator', 'type(dict().__iter__())'),
        ('dict_valueiterator', 'type(dict().values().__iter__())'),
        ('dict_itemiterator', 'type(dict().items().__iter__())'),
        ('dict_values', 'type(dict().values())'),
        ('dict_items', 'type(dict().items())'),
        ('set_iterator', 'type(set().__iter__())'),
        ('list_iterator', 'type(list().__iter__())'),
        ('range_iterator', 'type(range(0).__iter__())'),
        ('str_iterator', 'type(str().__iter__())'),
        ('tuple_iterator', 'type(tuple().__iter__())'),
    ]

    output.write(header.read())
    output.write("expected_methods = {\n")

    for name, typ_code in objects + iters:
        typ = eval(typ_code)
        output.write(f" '{name}': ({typ_code}, [\n")
        output.write(
            "\n".join(
                f"    {attr!r},"
                for attr in dir(typ)
                if attr_is_not_inherited(typ, attr)
            )
        )
        output.write("\n ])," + ("\n" if objects[-1] == typ else "\n\n"))

    output.write("}\n\n")
    output.write(footer.read())


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


def get_module_methods(name):
    with warnings.catch_warnings():
        # ignore warnings caused by importing deprecated modules
        warnings.filterwarnings("ignore", category=DeprecationWarning)
        try:
            return set(dir(__import__(name))) if name not in ("this", "antigravity") else None
        except ModuleNotFoundError:
            return None
        except Exception as e:
            print("!!! {} skipped because {}: {}".format(name, type(e).__name__, str(e)))


def gen_modules(header, footer, output):
    output.write(header.read())

    # check name because modules listed have side effects on import,
    # e.g. printing something or opening a webpage
    modules = {mod_name: get_module_methods(mod_name) for mod_name in scan_modules()}

    print(
        f"""
cpymods = {modules!r}
libdir = {os.path.abspath("../Lib/").encode('utf8')!r}
""",
        file=output,
    )

    output.write(footer.read())


gen_funcs = {"methods": gen_methods, "modules": gen_modules}


for name, gen_func in gen_funcs.items():
    gen_func(
        header=open(f"generator/not_impl_{name}_header.txt"),
        footer=open(f"generator/not_impl_{name}_footer.txt"),
        output=open(f"snippets/whats_left_{name}.py", "w"),
    )
