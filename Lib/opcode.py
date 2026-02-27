
"""
opcode module - potentially shared between dis and other modules which
operate on bytecodes (e.g. peephole optimizers).
"""


__all__ = ["cmp_op", "stack_effect", "hascompare", "opname", "opmap",
           "HAVE_ARGUMENT", "EXTENDED_ARG", "hasarg", "hasconst", "hasname",
           "hasjump", "hasjrel", "hasjabs", "hasfree", "haslocal", "hasexc"]

import builtins
import _opcode
from _opcode import stack_effect

from _opcode_metadata import (_specializations, _specialized_opmap, opmap,  # noqa: F401
                              HAVE_ARGUMENT, MIN_INSTRUMENTED_OPCODE)  # noqa: F401
EXTENDED_ARG = opmap['EXTENDED_ARG']

opname = ['<%r>' % (op,) for op in range(max(opmap.values()) + 1)]
for m in (opmap, _specialized_opmap):
    for op, i in m.items():
        opname[i] = op

cmp_op = ('<', '<=', '==', '!=', '>', '>=')

# These lists are documented as part of the dis module's API
hasarg = [op for op in opmap.values() if _opcode.has_arg(op)]
hasconst = [op for op in opmap.values() if _opcode.has_const(op)]
hasname = [op for op in opmap.values() if _opcode.has_name(op)]
hasjump = [op for op in opmap.values() if _opcode.has_jump(op)]
hasjrel = hasjump  # for backward compatibility
hasjabs = []
hasfree = [op for op in opmap.values() if _opcode.has_free(op)]
haslocal = [op for op in opmap.values() if _opcode.has_local(op)]
hasexc = [op for op in opmap.values() if _opcode.has_exc(op)]


_intrinsic_1_descs = _opcode.get_intrinsic1_descs()
_intrinsic_2_descs = _opcode.get_intrinsic2_descs()
_special_method_names = _opcode.get_special_method_names()
_common_constants = [builtins.AssertionError, builtins.NotImplementedError,
                     builtins.tuple, builtins.all, builtins.any]
_nb_ops = _opcode.get_nb_ops()

hascompare = [opmap["COMPARE_OP"]]

_cache_format = {
}

_inline_cache_entries = {
    name : sum(value.values()) for (name, value) in _cache_format.items()
}
