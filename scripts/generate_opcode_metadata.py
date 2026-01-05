"""Generate _opcode_metadata.py for RustPython bytecode.

This file generates opcode metadata that is compatible with CPython 3.13.
RustPython's Instruction enum is now ordered to match CPython opcode numbers exactly.
"""

import re

# Read the bytecode.rs file to get instruction names
with open("crates/compiler-core/src/bytecode.rs", "r") as f:
    content = f.read()

# Find the Instruction enum
match = re.search(r"pub enum Instruction \{(.+?)\n\}", content, re.DOTALL)
if not match:
    raise ValueError("Could not find Instruction enum")

enum_body = match.group(1)

# Extract variant names
variants = []
for line in enum_body.split("\n"):
    if line.strip().startswith("///") or line.strip().startswith("//"):
        continue
    m = re.match(r"^\s+([A-Z][a-zA-Z0-9]*)", line)
    if m:
        variants.append(m.group(1))

print(f"Found {len(variants)} instruction variants")

# Map RustPython variant names to CPython-compatible names
# The opcode number is the index in the Instruction enum
name_mapping = {
    # Dummy/placeholder instructions
    "Cache": "CACHE",
    "Reserved3": "RESERVED",
    "Reserved17": "RESERVED",
    "Reserved141": "RESERVED",
    "Reserved142": "RESERVED",
    "Reserved143": "RESERVED",
    "Reserved144": "RESERVED",
    "Reserved145": "RESERVED",
    "Reserved146": "RESERVED",
    "Reserved147": "RESERVED",
    "Reserved148": "RESERVED",
    "BinarySlice": "BINARY_SLICE",
    "EndFor": "END_FOR",
    "ExitInitCheck": "EXIT_INIT_CHECK",
    "GetYieldFromIter": "GET_YIELD_FROM_ITER",
    "InterpreterExit": "INTERPRETER_EXIT",
    "LoadAssertionError": "LOAD_ASSERTION_ERROR",
    "LoadLocals": "LOAD_LOCALS",
    "PushNull": "PUSH_NULL",
    "ReturnGenerator": "RETURN_GENERATOR",
    "StoreSlice": "STORE_SLICE",
    "UnaryInvert": "UNARY_INVERT",
    "UnaryNegative": "UNARY_NEGATIVE",
    "UnaryNot": "UNARY_NOT",
    "BuildConstKeyMap": "BUILD_CONST_KEY_MAP",
    "CopyFreeVars": "COPY_FREE_VARS",
    "DictMerge": "DICT_MERGE",
    "EnterExecutor": "ENTER_EXECUTOR",
    "JumpBackward": "JUMP_BACKWARD",
    "JumpBackwardNoInterrupt": "JUMP_BACKWARD_NO_INTERRUPT",
    "JumpForward": "JUMP_FORWARD",
    "ListExtend": "LIST_EXTEND",
    "LoadFastCheck": "LOAD_FAST_CHECK",
    "LoadFastLoadFast": "LOAD_FAST_LOAD_FAST",
    "LoadFromDictOrDeref": "LOAD_FROM_DICT_OR_DEREF",
    "LoadFromDictOrGlobals": "LOAD_FROM_DICT_OR_GLOBALS",
    "LoadSuperAttr": "LOAD_SUPER_ATTR",
    "MakeCell": "MAKE_CELL",
    "PopJumpIfNone": "POP_JUMP_IF_NONE",
    "PopJumpIfNotNone": "POP_JUMP_IF_NOT_NONE",
    "SetUpdate": "SET_UPDATE",
    "StoreFastStoreFast": "STORE_FAST_STORE_FAST",
    # Real instructions
    "BeforeAsyncWith": "BEFORE_ASYNC_WITH",
    "BeforeWith": "BEFORE_WITH",
    "BinaryOp": "BINARY_OP",
    "BinarySubscript": "BINARY_SUBSCR",
    "Break": "BREAK",
    "BuildList": "BUILD_LIST",
    "BuildListFromTuples": "BUILD_LIST_UNPACK",
    "BuildMap": "BUILD_MAP",
    "BuildMapForCall": "BUILD_MAP_FOR_CALL",
    "BuildSet": "BUILD_SET",
    "BuildSetFromTuples": "BUILD_SET_UNPACK",
    "BuildSlice": "BUILD_SLICE",
    "BuildString": "BUILD_STRING",
    "BuildTuple": "BUILD_TUPLE",
    "BuildTupleFromIter": "BUILD_TUPLE_ITER",
    "BuildTupleFromTuples": "BUILD_TUPLE_UNPACK",
    "CallFunctionEx": "CALL_FUNCTION_EX",
    "CallFunctionKeyword": "CALL_KW",
    "CallFunctionPositional": "CALL",
    "CallIntrinsic1": "CALL_INTRINSIC_1",
    "CallIntrinsic2": "CALL_INTRINSIC_2",
    "CallMethodEx": "CALL_METHOD_EX",
    "CallMethodKeyword": "CALL_METHOD_KW",
    "CallMethodPositional": "CALL_METHOD",
    "CheckEgMatch": "CHECK_EG_MATCH",
    "CheckExcMatch": "CHECK_EXC_MATCH",
    "CleanupThrow": "CLEANUP_THROW",
    "CompareOperation": "COMPARE_OP",
    "ContainsOp": "CONTAINS_OP",
    "Continue": "CONTINUE",
    "ConvertValue": "CONVERT_VALUE",
    "CopyItem": "COPY",
    "DeleteAttr": "DELETE_ATTR",
    "DeleteDeref": "DELETE_DEREF",
    "DeleteFast": "DELETE_FAST",
    "DeleteGlobal": "DELETE_GLOBAL",
    "DeleteLocal": "DELETE_NAME",
    "DeleteSubscript": "DELETE_SUBSCR",
    "DictUpdate": "DICT_UPDATE",
    "EndAsyncFor": "END_ASYNC_FOR",
    "EndSend": "END_SEND",
    "ExtendedArg": "EXTENDED_ARG",
    "ForIter": "FOR_ITER",
    "FormatSimple": "FORMAT_SIMPLE",
    "FormatWithSpec": "FORMAT_WITH_SPEC",
    "GetAIter": "GET_AITER",
    "GetANext": "GET_ANEXT",
    "GetAwaitable": "GET_AWAITABLE",
    "GetIter": "GET_ITER",
    "GetLen": "GET_LEN",
    "ImportFrom": "IMPORT_FROM",
    "ImportName": "IMPORT_NAME",
    "IsOp": "IS_OP",
    "Jump": "JUMP",
    "JumpIfFalseOrPop": "JUMP_IF_FALSE_OR_POP",
    "JumpIfNotExcMatch": "JUMP_IF_NOT_EXC_MATCH",
    "JumpIfTrueOrPop": "JUMP_IF_TRUE_OR_POP",
    "ListAppend": "LIST_APPEND",
    "LoadAttr": "LOAD_ATTR",
    "LoadBuildClass": "LOAD_BUILD_CLASS",
    "LoadClassDeref": "LOAD_CLASSDEREF",
    "LoadClosure": "LOAD_CLOSURE",
    "LoadConst": "LOAD_CONST",
    "LoadDeref": "LOAD_DEREF",
    "LoadFast": "LOAD_FAST",
    "LoadFastAndClear": "LOAD_FAST_AND_CLEAR",
    "LoadGlobal": "LOAD_GLOBAL",
    "LoadMethod": "LOAD_METHOD",
    "LoadNameAny": "LOAD_NAME",
    "MakeFunction": "MAKE_FUNCTION",
    "MapAdd": "MAP_ADD",
    "MatchClass": "MATCH_CLASS",
    "MatchKeys": "MATCH_KEYS",
    "MatchMapping": "MATCH_MAPPING",
    "MatchSequence": "MATCH_SEQUENCE",
    "Nop": "NOP",
    "PopBlock": "POP_BLOCK",
    "PopException": "POP_EXCEPT",
    "PopJumpIfFalse": "POP_JUMP_IF_FALSE",
    "PopJumpIfTrue": "POP_JUMP_IF_TRUE",
    "PopTop": "POP_TOP",
    "PushExcInfo": "PUSH_EXC_INFO",
    "Raise": "RAISE_VARARGS",
    "Reraise": "RERAISE",
    "Resume": "RESUME",
    "ReturnConst": "RETURN_CONST",
    "ReturnValue": "RETURN_VALUE",
    "Reverse": "REVERSE",
    "Send": "SEND",
    "SetAdd": "SET_ADD",
    "SetExcInfo": "SET_EXC_INFO",
    "SetFunctionAttribute": "SET_FUNCTION_ATTRIBUTE",
    "SetupAnnotation": "SETUP_ANNOTATIONS",
    "StoreAttr": "STORE_ATTR",
    "StoreDeref": "STORE_DEREF",
    "StoreFast": "STORE_FAST",
    "StoreFastLoadFast": "STORE_FAST_LOAD_FAST",
    "StoreGlobal": "STORE_GLOBAL",
    "StoreLocal": "STORE_NAME",
    "StoreSubscript": "STORE_SUBSCR",
    "Subscript": "SUBSCRIPT",
    "Swap": "SWAP",
    "ToBool": "TO_BOOL",
    "UnaryOperation": "UNARY_OP",
    "UnpackEx": "UNPACK_EX",
    "UnpackSequence": "UNPACK_SEQUENCE",
    "WithExceptStart": "WITH_EXCEPT_START",
    "YieldValue": "YIELD_VALUE",
}

# Build opmap with RustPython instruction indices
opmap = {}
rust_to_cpython_name = {}
for i, variant in enumerate(variants):
    cpython_name = name_mapping.get(variant, variant.upper())
    # Skip adding duplicates (RESERVED appears multiple times)
    if cpython_name == "RESERVED":
        # Use unique names for reserved slots
        cpython_name = f"RESERVED_{i}"
    if cpython_name not in opmap:
        opmap[cpython_name] = i
    rust_to_cpython_name[variant] = cpython_name


# Find specific instruction indices for categorization
def find_opcode(cpython_name):
    return opmap.get(cpython_name, -1)


# Generate the output file
output = """# This file is generated by scripts/generate_opcode_metadata.py
# for RustPython bytecode format (CPython 3.13 compatible opcode numbers).
# Do not edit!

_specializations = {}

_specialized_opmap = {}

opmap = {
"""

for name, num in sorted(opmap.items(), key=lambda x: x[1]):
    output += f"    '{name}': {num},\n"

output += """}

# CPython 3.13 compatible: opcodes < 44 have no argument
HAVE_ARGUMENT = 44
MIN_INSTRUMENTED_OPCODE = 236
"""

with open("Lib/_opcode_metadata.py", "w") as f:
    f.write(output)

print("Generated Lib/_opcode_metadata.py")
print("\nKey opcode indices (matching CPython 3.13):")
print(f"  CACHE = {find_opcode('CACHE')} (expected: 0)")
print(f"  BEFORE_ASYNC_WITH = {find_opcode('BEFORE_ASYNC_WITH')} (expected: 1)")
print(f"  BINARY_SUBSCR = {find_opcode('BINARY_SUBSCR')} (expected: 5)")
print(
    f"  WITH_EXCEPT_START = {find_opcode('WITH_EXCEPT_START')} (expected: 44, HAVE_ARGUMENT)"
)
print(f"  BINARY_OP = {find_opcode('BINARY_OP')} (expected: 45)")
print(f"  LOAD_CONST = {find_opcode('LOAD_CONST')} (expected: 83)")
print(f"  LOAD_FAST = {find_opcode('LOAD_FAST')} (expected: 85)")
print(f"  LOAD_GLOBAL = {find_opcode('LOAD_GLOBAL')} (expected: 91)")
print(f"  STORE_FAST = {find_opcode('STORE_FAST')} (expected: 110)")
print(f"  YIELD_VALUE = {find_opcode('YIELD_VALUE')} (expected: 118)")
print(f"  RESUME = {find_opcode('RESUME')} (expected: 149)")
