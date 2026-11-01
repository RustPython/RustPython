import builtins
import importlib._bootstrap as bootstrap
import sys
import types

# Native imports pass the original builtin import function back to importlib.
# Passing bootstrap.__import__ instead re-enters Python's module-lock manager
# when a concurrent circular import needs an already-initializing parent.
original_import = builtins.__import__
original_find_and_load = bootstrap._find_and_load
original_handle_fromlist = bootstrap._handle_fromlist
module_name = "_canonical_import_callback_probe"
module = types.ModuleType(module_name)
module.__path__ = []
callbacks = []
overrides = []


def find_and_load(name, import_, **kwargs):
    if name == module_name:
        callbacks.append(import_)
        return module
    return original_find_and_load(name, import_, **kwargs)


def handle_fromlist(module_, fromlist, import_, **kwargs):
    if module_ is module:
        callbacks.append(import_)
        return module
    return original_handle_fromlist(module_, fromlist, import_, **kwargs)


def import_statement():
    import _canonical_import_callback_probe

    return _canonical_import_callback_probe


def override_import(name, *args, **kwargs):
    if name == module_name:
        overrides.append(name)
    return original_import(name, *args, **kwargs)


bootstrap._find_and_load = find_and_load
bootstrap._handle_fromlist = handle_fromlist
try:
    for do_import in (import_statement, lambda: original_import(module_name)):
        assert do_import() is module
        assert callbacks.pop() is original_import

    assert original_import(module_name, fromlist=("child",)) is module
    assert callbacks.pop() is original_import  # _handle_fromlist
    assert callbacks.pop() is original_import  # _find_and_load

    builtins.__import__ = override_import
    assert import_statement() is module
    assert overrides.pop() == module_name
    assert callbacks.pop() is original_import

    # A cache hit must not bypass a user replacement of builtins.__import__.
    sys.modules[module_name] = module
    assert import_statement() is module
    assert overrides.pop() == module_name
    assert callbacks == []
    assert overrides == []
finally:
    builtins.__import__ = original_import
    bootstrap._find_and_load = original_find_and_load
    bootstrap._handle_fromlist = original_handle_fromlist
    sys.modules.pop(module_name, None)
