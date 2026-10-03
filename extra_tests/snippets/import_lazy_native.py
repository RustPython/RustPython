"""PEP 810 bindings, importer callbacks, raw dictionaries, and warm-cache behavior."""

import os
import pathlib
import subprocess
import sys
import tempfile
import textwrap

CASES = {
    "explicit_declaration_is_deferred": r"""
    lazy import p810_fixture
    assert 'p810_fixture' not in sys.modules
    assert type(globals()['p810_fixture']).__name__ == 'lazy_import'
    assert p810_fixture.VALUE == 42
    assert type(globals()['p810_fixture']).__name__ == 'module'
""",
    "compatibility_from_is_deferred": r"""
    __lazy_modules__ = ['p810_fixture']
    from p810_fixture import VALUE, other
    assert 'p810_fixture' not in sys.modules
    assert VALUE == 42
    assert type(globals()['other']).__name__ == 'lazy_import'
    assert other() == 43
""",
    "raw_dict_and_resolve": r"""
    lazy import p810_fixture
    assert type(globals()['p810_fixture']).__name__ == 'lazy_import'
    assert 'p810_fixture' in dir()
    assert 'p810_fixture' not in sys.modules
    assert globals()['p810_fixture'].resolve().VALUE == 42
    assert type(globals()['p810_fixture']).__name__ == 'lazy_import'
    assert p810_fixture.VALUE == 42
    assert type(globals()['p810_fixture']).__name__ == 'module'
""",
    "function_global_writeback": r"""
    lazy import p810_fixture
    def f(): return p810_fixture.VALUE
    assert 'p810_fixture' not in sys.modules
    assert f() == 42
    assert type(globals()['p810_fixture']).__name__ == 'module'
""",
    "function_local_proxy_stays_raw": r"""
    lazy import p810_fixture
    def f():
        proxy = globals()['p810_fixture']
        assert type(proxy).__name__ == 'lazy_import'
        assert 'p810_fixture' not in sys.modules
        return proxy.resolve().VALUE
    assert f() == 42
""",
    "dotted_package_access": r"""
    lazy import p810_pkg.leaf
    assert 'p810_pkg' not in sys.modules
    assert p810_pkg.leaf.VALUE == 42
    assert sys.modules['p810_pkg'].__dict__['leaf'] is sys.modules['p810_pkg.leaf']
""",
    "dotted_alias_access": r"""
    lazy import p810_pkg.leaf as leaf
    assert 'p810_pkg' not in sys.modules
    assert leaf.VALUE == 42
""",
    "relative_compatibility": r"""
    import p810_pkg.consumer as consumer
    assert 'p810_pkg.leaf' not in sys.modules
    assert consumer.get() == 42
""",
    "module_attr_cache_replacement": r"""
    import types
    m = types.ModuleType('probe')
    m.value = 1
    def f(): return m.value
    for _ in range(200): assert f() == 1
    lazy from p810_fixture import VALUE
    m.__dict__['value'] = globals()['VALUE']
    assert type(m.__dict__['value']).__name__ == 'lazy_import'
    assert f() == 42
    assert m.__dict__['value'] == 42
""",
    "global_cache_replacement": r"""
    value = 1
    def f(): return value
    for _ in range(200): assert f() == 1
    lazy from p810_fixture import VALUE
    globals()['value'] = globals()['VALUE']
    assert f() == 42
    assert type(globals()['VALUE']).__name__ == 'lazy_import'
""",
    "builtins_cache_replacement": r"""
    import builtins
    builtins.p810_value = 1
    def f(): return p810_value
    for _ in range(200): assert f() == 1
    lazy from p810_fixture import VALUE
    builtins.__dict__['p810_value'] = globals()['VALUE']
    assert f() == 42
    assert globals()['p810_value'] == 42
    assert type(builtins.__dict__['p810_value']).__name__ == 'lazy_import'
""",
    "deferred_error_retries": r"""
    lazy import p810_broken
    for _ in range(2):
        try: p810_broken
        except ValueError as error:
            assert str(error) == 'expected deferred failure'
            assert error.__cause__ is not None
        else: raise AssertionError('not deferred')
        assert 'p810_broken' not in sys.modules
        assert type(globals()['p810_broken']).__name__ == 'lazy_import'
""",
    "custom_import_lookup_at_resolution": r"""
    import builtins
    ns = {'__name__':'probe', '__builtins__':dict(builtins.__dict__)}
    exec('lazy import p810_fixture\ndef f(): return p810_fixture', ns)
    seen = []
    def hook(name, glob, loc, fromlist, level):
        seen.append((name, level, glob is globals(), loc is globals()))
        return 123
    ns['__builtins__']['__import__'] = hook
    assert ns['p810_fixture'].resolve() == 123
    assert seen == [('p810_fixture', 0, True, True)]
""",
    "custom_lazy_import_callback": r"""
    import builtins
    ns = {'__name__':'probe', '__builtins__':dict(builtins.__dict__)}
    seen = []
    def hook(*args):
        seen.append(args)
        return 123
    ns['__builtins__']['__lazy_import__'] = hook
    exec('lazy import p810_fixture', ns)
    assert ns['p810_fixture'] == 123
    assert len(seen[0]) == 6
    assert seen[0][0] == 'p810_fixture'
    assert seen[0][1] is ns and seen[0][2] is ns
    assert seen[0][5] is ns['__builtins__']
""",
    "function_class_try_forced_eager": r"""
    __lazy_modules__ = ['p810_fixture']
    def f():
        import p810_fixture
    f()
    assert 'p810_fixture' in sys.modules
    del sys.modules['p810_fixture']
    class C:
        import p810_fixture
    assert 'p810_fixture' in sys.modules
    del sys.modules['p810_fixture']
    try:
        import p810_fixture
    except ImportError:
        raise
    assert 'p810_fixture' in sys.modules
""",
    "explicit_scope_restrictions": r"""
    for src in [
        'def f():\n lazy import p810_fixture',
        'class C:\n lazy import p810_fixture',
        'try:\n lazy import p810_fixture\nexcept: pass',
        'lazy from p810_fixture import *',
        'lazy from __future__ import annotations',
    ]:
        try: compile(src, '<probe>', 'exec')
        except SyntaxError: pass
        else: raise AssertionError(src)
""",
    "exec_context_restrictions": r"""
    def f(): exec('lazy import p810_fixture')
    try: f()
    except SyntaxError: pass
    else: raise AssertionError('function exec admitted')
    ns = {'__name__':'probe'}
    exec('lazy import p810_fixture', ns)
    assert type(ns['p810_fixture']).__name__ == 'lazy_import'
""",
    "ast_roundtrip_is_lazy": r"""
    import ast
    tree = ast.parse('lazy import p810_fixture')
    assert tree.body[0].is_lazy == 1
    assert 'is_lazy' in tree.body[0]._fields
    ns = {'__name__':'probe'}
    exec(compile(tree, '<probe>', 'exec'), ns)
    assert 'p810_fixture' not in sys.modules
""",
    "module_getattr_precedes_proxy": r"""
    import types
    m = types.ModuleType('probe')
    exec('lazy from p810_fixture import VALUE', m.__dict__)
    m.__getattr__ = lambda name: 99
    assert m.VALUE == 99
    assert 'p810_fixture' not in sys.modules
    assert type(m.__dict__['VALUE']).__name__ == 'lazy_import'
""",
    "registry_cleanup": r"""
    lazy import p810_fixture
    assert 'p810_fixture' in sys.lazy_modules
    assert p810_fixture.VALUE == 42
    assert 'p810_fixture' not in sys.lazy_modules
""",
    "public_api_argument_contract": r"""
    for action, error in [
        (lambda: __lazy_import__('p810_fixture', None), TypeError),
        (lambda: __lazy_import__('p810_fixture', {}), ValueError),
        (lambda: __lazy_import__('p810_fixture', {}, None, (), 0, {}), TypeError),
    ]:
        try: action()
        except error: pass
        else: raise AssertionError('public argument contract')
""",
    "public_api_captures_supplied_builtins": r"""
    import builtins
    class D(dict):
        def __getitem__(self, key):
            if key == '__builtins__': raise AssertionError('must read raw globals')
            return super().__getitem__(key)
    seen = []
    def hook(name, glob, loc, fromlist, level):
        seen.append(name)
        return 812
    custom = dict(builtins.__dict__, __import__=hook)
    holder = []
    holder.append(__lazy_import__('p810_fixture', D(__builtins__=custom)))
    assert holder[0].resolve() == 812
    assert seen == ['p810_fixture']
""",
    "dict_subclass_raw_global_writeback": r"""
    import builtins
    lazy import p810_fixture
    class D(dict):
        def __setitem__(self, key, value): raise AssertionError('must write raw dict')
    ns = D(__builtins__=builtins.__dict__, p810_fixture=globals()['p810_fixture'])
    assert eval('p810_fixture.VALUE', ns) == 42
    assert type(dict.__getitem__(ns, 'p810_fixture')).__name__ == 'module'
""",
    "filter_reads_raw_importer_name": r"""
    import builtins
    class D(dict):
        def __getitem__(self, key):
            if key == '__name__': raise AssertionError('must read raw importer name')
            return super().__getitem__(key)
    seen=[]
    sys.set_lazy_imports_filter(lambda *args: seen.append(args) or True)
    holder=[]
    holder.append(__lazy_import__('p810_fixture', D(__name__='stored', __builtins__=builtins.__dict__)))
    assert seen[0][0] == 'stored'
    assert type(holder[0]).__name__ == 'lazy_import'
""",
    "user_module_cycle_error_propagates": r"""
    import types
    m = types.ModuleType('p810_user_cycle')
    def f(name): raise ImportCycleError('user error')
    m.__getattr__ = f
    sys.modules[m.__name__] = m
    try: getattr(m, 'missing', None)
    except ImportCycleError as e: assert str(e) == 'user error'
    else: raise AssertionError('cycle error was swallowed')
    try: from p810_user_cycle import missing
    except ImportCycleError as e: assert str(e) == 'user error'
    else: raise AssertionError('cycle error was swallowed')
""",
    "existing_import_from_waits_initialization": r"""
    import types, importlib._bootstrap as bootstrap
    m = types.ModuleType('p810_partial')
    m.VALUE = 1
    m.__spec__ = types.SimpleNamespace(_initializing=True)
    sys.modules[m.__name__] = m
    seen=[]
    old = bootstrap._lock_unlock_module
    def unlock(name):
        seen.append(name)
        m.VALUE = 2
        m.__spec__._initializing = False
    bootstrap._lock_unlock_module = unlock
    try: exec('lazy from p810_partial import VALUE', globals())
    finally: bootstrap._lock_unlock_module = old
    assert seen == ['p810_partial']
    assert globals()['VALUE'] == 2
""",
    "existing_import_from_discards_stale_module": r"""
    import types, importlib._bootstrap as bootstrap
    m = types.ModuleType('p810_partial')
    m.VALUE = 1
    m.__spec__ = types.SimpleNamespace(_initializing=True)
    sys.modules[m.__name__] = m
    old = bootstrap._lock_unlock_module
    def unlock(name): del sys.modules[name]
    bootstrap._lock_unlock_module = unlock
    try: exec('lazy from p810_partial import VALUE', globals())
    finally: bootstrap._lock_unlock_module = old
    assert type(globals()['VALUE']).__name__ == 'lazy_import'
""",
    "frozen_global_reification_writeback": r"""
    import builtins
    lazy import p810_fixture
    frozen = frozendict(__builtins__=builtins.__dict__, p810_fixture=globals()['p810_fixture'])
    try: eval('p810_fixture', frozen)
    except TypeError as e: assert 'does not support item assignment' in str(e)
    else: raise AssertionError('frozen writeback did not fail')
    assert 'p810_fixture' in sys.modules
    assert type(frozen['p810_fixture']).__name__ == 'lazy_import'
    assert frozen['p810_fixture'].resolve().VALUE == 42
    out = {}
    exec('def f(): return p810_fixture', frozen, out)
    try: out['f']()
    except TypeError as e: assert 'does not support item assignment' in str(e)
    else: raise AssertionError('frozen function writeback did not fail')
""",
    "runtime_policy_and_filter": r"""
    sys.set_lazy_imports('all')
    import p810_fixture
    assert 'p810_fixture' not in sys.modules
    sys.set_lazy_imports('normal')
    assert p810_fixture.VALUE == 42
    del sys.modules['p810_fixture']
    seen=[]
    sys.set_lazy_imports_filter(lambda *args: seen.append(args) or False)
    lazy import p810_fixture
    assert 'p810_fixture' in sys.modules
    assert seen[0][1] == 'p810_fixture'
    try: sys.set_lazy_imports_filter(42)
    except ValueError: pass
    else: raise AssertionError('bad filter accepted')
    try: sys.set_lazy_imports('none')
    except ValueError: pass
    else: raise AssertionError('bad mode accepted')
""",
    "ast_optional_field_and_unparse": r"""
    import ast
    assert ast.Import(names=[]).is_lazy is None
    assert 'None' in str(ast.Import._field_types['is_lazy'])
    node = ast.Import(names=[ast.alias(name='p810_fixture')], is_lazy=2)
    tree = ast.fix_missing_locations(ast.Module(body=[node], type_ignores=[]))
    copied = compile(tree, '<test>', 'exec', ast.PyCF_ONLY_AST)
    assert copied.body[0].is_lazy == 2
    assert ast.unparse(ast.parse('lazy import p810_fixture')) == 'lazy import p810_fixture'
    try: ast.parse('lazy from __future__ import annotations')
    except SyntaxError as error: assert error.offset == 1 and error.end_offset == 5
    else: raise AssertionError('lazy future parsed')
""",
}

with tempfile.TemporaryDirectory(prefix="rustpython-lazy-") as root:
    fixture = pathlib.Path(root)
    (fixture / "p810_fixture.py").write_text("VALUE=42\ndef other(): return 43\n")
    (fixture / "p810_broken.py").write_text(
        "raise ValueError('expected deferred failure')\n"
    )
    package = fixture / "p810_pkg"
    package.mkdir()
    (package / "__init__.py").write_text("")
    (package / "leaf.py").write_text("VALUE=42\n")
    (package / "consumer.py").write_text(
        "__lazy_modules__=['p810_pkg.leaf']\nfrom .leaf import VALUE\ndef get(): return VALUE\n"
    )
    for name, body in CASES.items():
        code = (
            "import sys\nsys.path.insert(0, "
            + repr(root)
            + ")\n"
            + textwrap.dedent(body)
        )
        result = subprocess.run(
            [sys.executable, "-B", "-S", "-c", code],
            capture_output=True,
            text=True,
            timeout=30,
            env=dict(os.environ, PYTHONDONTWRITEBYTECODE="1"),
        )
        assert result.returncode == 0, (name, result.stdout, result.stderr)
