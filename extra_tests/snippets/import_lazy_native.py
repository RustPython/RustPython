"""Lazy reification honors custom mappings and read-only namespaces."""

import os
import pathlib
import subprocess
import sys
import tempfile
import textwrap

CASES = {
    "dict_subclass_mapping_writeback": r"""
    import builtins
    lazy import p810_fixture
    class D(dict):
        def __setitem__(self, key, value): raise ValueError('custom writeback')
    ns = D(__builtins__=builtins.__dict__, p810_fixture=globals()['p810_fixture'])
    try: eval('p810_fixture.VALUE', ns)
    except ValueError as error: assert str(error) == 'custom writeback'
    else: raise AssertionError('mapping writeback bypassed the subclass')
    assert type(dict.__getitem__(ns, 'p810_fixture')).__name__ == 'lazy_import'
""",
    "frozen_global_reification_without_writeback": r"""
    import builtins
    lazy import p810_fixture
    frozen = frozendict(__builtins__=builtins.__dict__, p810_fixture=globals()['p810_fixture'])
    assert eval('p810_fixture', frozen) is sys.modules['p810_fixture']
    assert type(frozen['p810_fixture']).__name__ == 'lazy_import'
    assert frozen['p810_fixture'].resolve().VALUE == 42
    out = {}
    exec('def f(): return p810_fixture', frozen, out)
    assert out['f']() is sys.modules['p810_fixture']
    assert type(frozen['p810_fixture']).__name__ == 'lazy_import'
""",
}

with tempfile.TemporaryDirectory(prefix="rustpython-lazy-") as root:
    fixture = pathlib.Path(root)
    (fixture / "p810_fixture.py").write_text("VALUE=42\n")
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
