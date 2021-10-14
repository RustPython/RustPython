from typing import Type
from types import (
    GeneratorType, CoroutineType, AsyncGeneratorType, BuiltinFunctionType,
    BuiltinMethodType, WrapperDescriptorType, MethodWrapperType, MethodDescriptorType,
    ClassMethodDescriptorType, FrameType, GetSetDescriptorType, MemberDescriptorType
)
from testutils import assert_raises

def check_forbidden_instantiation(typ, reverse=False):
    f = reversed if reverse else iter
    with assert_raises(TypeError):
        type(f(typ()))()

dict_values, dict_items = lambda: {}.values(), lambda: {}.items()
# types with custom forward iterators
iter_types = [list, set, str, bytearray, bytes, dict, tuple, lambda: range(0), dict_items, dict_values]
# types with custom backwards iterators
reviter_types = [list, dict, lambda: range(0), dict_values, dict_items]
# internal types:
internal_types = [
    GeneratorType,
    CoroutineType,
    AsyncGeneratorType,
    BuiltinFunctionType,
    BuiltinMethodType, # same as MethodWrapperType 
    WrapperDescriptorType,
    MethodWrapperType,
    MethodDescriptorType,
    ClassMethodDescriptorType,
    FrameType,
    GetSetDescriptorType, # same as MemberDescriptorType 
    MemberDescriptorType
]

for typ in iter_types:
    check_forbidden_instantiation(typ)
for typ in reviter_types:
    check_forbidden_instantiation(typ, reverse=True)
for typ in internal_types:
    with assert_raises(TypeError):
        typ()