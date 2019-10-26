# It's recommended to run this with `python3 -I not_impl_gen.py`, to make sure
# that nothing in your global Python environment interferes with what's being
# extracted here.

import pkgutil
import os
import sys
import warnings

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
    objects = [
        bool,
        bytearray,
        bytes,
        complex,
        dict,
        float,
        frozenset,
        int,
        list,
        memoryview,
        range,
        set,
        str,
        tuple,
        object,
    ]

    output.write(header.read())
    output.write("expected_methods = {\n")

    for obj in objects:
        output.write(f" '{obj.__name__}': ({obj.__name__}, [\n")
        output.write(
            "\n".join(
                f"    {attr!r},"
                for attr in dir(obj)
                if attr_is_not_inherited(obj, attr)
            )
        )
        output.write("\n ])," + ("\n" if objects[-1] == obj else "\n\n"))

    output.write("}\n\n")
    output.write(footer.read())

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

    modules = dict(
        map(
            lambda mod: (
                mod.name,
                # check name b/c modules listed have side effects on import,
                # e.g. printing something or opening a webpage
                get_module_methods(mod.name)
            ),
            pkgutil.iter_modules(),
        )
    )

    print(
        f"""
cpymods = {modules!r}
libdir = {os.path.abspath("../Lib/")!r}
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

