import sys
import warnings
from pydoc import ModuleScanner


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


def traverse(module, names, item):
    import inspect
    has_doc = inspect.ismodule(item) or inspect.isclass(item) or inspect.isbuiltin(item)
    if has_doc and isinstance(item.__doc__, str):
        yield names, item.__doc__
    attr_names = dir(item)
    for name in attr_names:
        if name in ['__class__', '__dict__', '__doc__', '__objclass__', '__name__', '__qualname__']:
            continue
        try:
            attr = getattr(item, name)
        except AttributeError:
            assert name == '__abstractmethods__'
            continue

        if module is item and not is_child(module, attr):
            continue

        is_type_or_module = (type(attr) is type) or (type(attr) is type(__builtins__))
        new_names = names.copy()
        new_names.append(name)

        if item == attr:
            pass
        elif not inspect.ismodule(item) and inspect.ismodule(attr):
            pass
        elif is_type_or_module:
            yield from traverse(module, new_names, attr)
        elif callable(attr) or not issubclass(type(attr), type) or type(attr).__name__ in ('getset_descriptor', 'member_descriptor'):
            if inspect.isbuiltin(attr):
                yield new_names, attr.__doc__
        else:
            assert False, (module, new_names, attr, type(attr).__name__)


def traverse_all():
    for module_name in scan_modules():
        if module_name in ('this', 'antigravity'):
            continue
        module = import_module(module_name)
        if hasattr(module, '__cached__'):  # python module
            continue
        yield from traverse(module, [module_name], module)


def docs():
    docs = {'.'.join(names): doc for names, doc in traverse_all()}
    return docs

if __name__ == '__main__':
    import json
    print(json.dumps(docs(), indent=4, sort_keys=True))
