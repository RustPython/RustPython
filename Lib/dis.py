import types # XXX: From CPython 3.13.7
from _dis import *

# XXX: From CPython 3.13.7
from opcode import *
# XXX: From CPython 3.13.7
from opcode import (
    __all__ as _opcodes_all,
    _cache_format,
    _inline_cache_entries,
    _nb_ops,
    _intrinsic_1_descs,
    _intrinsic_2_descs,
    _specializations,
    _specialized_opmap,
)

# XXX: From CPython 3.13.7
from _opcode import get_executor

# XXX: From CPython 3.13.7
__all__ = ["code_info", "dis", "disassemble", "distb", "disco",
           "findlinestarts", "findlabels", "show_code",
           "get_instructions", "Instruction", "Bytecode"] + _opcodes_all
del _opcodes_all

# XXX: From CPython 3.13.7
_have_code = (types.MethodType, types.FunctionType, types.CodeType,
              classmethod, staticmethod, type)

# XXX: From CPython 3.13.7
CONVERT_VALUE = opmap['CONVERT_VALUE']

# XXX: From CPython 3.13.7
SET_FUNCTION_ATTRIBUTE = opmap['SET_FUNCTION_ATTRIBUTE']
FUNCTION_ATTR_FLAGS = ('defaults', 'kwdefaults', 'annotations', 'closure')

# XXX: From CPython 3.13.7
ENTER_EXECUTOR = opmap['ENTER_EXECUTOR']
LOAD_CONST = opmap['LOAD_CONST']
RETURN_CONST = opmap['RETURN_CONST']
LOAD_GLOBAL = opmap['LOAD_GLOBAL']
BINARY_OP = opmap['BINARY_OP']
JUMP_BACKWARD = opmap['JUMP_BACKWARD']
FOR_ITER = opmap['FOR_ITER']
SEND = opmap['SEND']
LOAD_ATTR = opmap['LOAD_ATTR']
LOAD_SUPER_ATTR = opmap['LOAD_SUPER_ATTR']
CALL_INTRINSIC_1 = opmap['CALL_INTRINSIC_1']
CALL_INTRINSIC_2 = opmap['CALL_INTRINSIC_2']
LOAD_FAST_LOAD_FAST = opmap['LOAD_FAST_LOAD_FAST']
STORE_FAST_LOAD_FAST = opmap['STORE_FAST_LOAD_FAST']
STORE_FAST_STORE_FAST = opmap['STORE_FAST_STORE_FAST']

# XXX: From CPython 3.13.7
CACHE = opmap["CACHE"]

# XXX: From CPython 3.13.7
_all_opname = list(opname)
_all_opmap = dict(opmap)
for name, op in _specialized_opmap.items():
    # fill opname and opmap
    assert op < len(_all_opname)
    _all_opname[op] = name
    _all_opmap[name] = op

# XXX: From CPython 3.13.7
deoptmap = {
    specialized: base for base, family in _specializations.items() for specialized in family
}

# Disassembling a file by following cpython Lib/dis.py
def _test():
    """Simple test program to disassemble a file."""
    import argparse

    parser = argparse.ArgumentParser()
    parser.add_argument('infile', type=argparse.FileType('rb'), nargs='?', default='-')
    args = parser.parse_args()
    with args.infile as infile:
        source = infile.read()
    code = compile(source, args.infile.name, "exec")
    dis(code)

if __name__ == "__main__":
    _test()
