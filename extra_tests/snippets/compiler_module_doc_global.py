# Regression test: a module docstring must be stored through module name
# resolution, so a nested `global __doc__` declaration makes the module body
# emit STORE_GLOBAL (writing module globals) instead of STORE_NAME (writing
# the provided locals mapping). Mirrors CPython codegen_nameop behavior.

import dis


def opnames(code):
    return [instr.opname for instr in dis.get_instructions(code)]


def run_split(src, optimize=0):
    code = compile(src, "<test>", "exec", optimize=optimize)
    g, loc = {}, {}
    exec(code, g, loc)
    return code, g, loc


# A nested function declaring `global __doc__` makes the module docstring
# store target module globals.
src_global = '"module doc"\ndef f():\n    global __doc__\n'
code, g, loc = run_split(src_global)
ops = opnames(code)
assert "STORE_GLOBAL" in ops, ops
assert "STORE_NAME" not in ops or "__doc__" not in [
    instr.argval for instr in dis.get_instructions(code) if instr.opname == "STORE_NAME"
], ops
assert g.get("__doc__") == "module doc", g
assert "__doc__" not in loc, loc

# The declaration may be nested more than one scope deep.
src_deep = '"module doc"\ndef f():\n    def g():\n        global __doc__\n'
code, g, loc = run_split(src_deep)
assert "STORE_GLOBAL" in opnames(code), opnames(code)
assert g.get("__doc__") == "module doc", g
assert "__doc__" not in loc, loc

# Control: without a nested global declaration the docstring stays a
# STORE_NAME and lands in the provided locals mapping.
src_plain = '"module doc"\nx = 1\n'
code, g, loc = run_split(src_plain)
doc_stores = [
    instr.opname
    for instr in dis.get_instructions(code)
    if instr.argval == "__doc__" and instr.opname.startswith("STORE_")
]
assert doc_stores == ["STORE_NAME"], doc_stores
assert loc.get("__doc__") == "module doc", loc
assert "__doc__" not in g, g

# optimize=1 keeps the docstring and the same global resolution.
code, g, loc = run_split(src_global, optimize=1)
assert "STORE_GLOBAL" in opnames(code), opnames(code)
assert g.get("__doc__") == "module doc", g

# optimize=2 strips the docstring entirely: no __doc__ store in either dict.
code, g, loc = run_split(src_global, optimize=2)
assert not any(
    instr.argval == "__doc__" and instr.opname.startswith("STORE_")
    for instr in dis.get_instructions(code)
), opnames(code)
assert "__doc__" not in g and "__doc__" not in loc, (g, loc)

# A class docstring still belongs to the class namespace even when a nested
# scope declares `global __doc__`.
src_class = '"module doc"\ndef f():\n    global __doc__\nclass C:\n    "class doc"\n'
g, loc = {}, {}
exec(compile(src_class, "<test>", "exec"), g, loc)
assert g.get("__doc__") == "module doc", g
assert loc["C"].__doc__ == "class doc", loc["C"].__doc__
assert "__doc__" not in loc, loc
