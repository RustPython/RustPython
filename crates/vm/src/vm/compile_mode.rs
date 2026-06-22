use crate::bytecode;

pub(crate) const PY_SINGLE_INPUT: i32 = 256;
pub(crate) const PY_FILE_INPUT: i32 = 257;
pub(crate) const PY_EVAL_INPUT: i32 = 258;
pub(crate) const PY_FUNC_TYPE_INPUT: i32 = 345;

// Caveat emptor: These flags are undocumented on purpose and depending
// on their effect outside the standard library is **unsupported**.
pub(crate) const PY_CF_SOURCE_IS_UTF8: i32 = 0x0100;
pub(crate) const PY_CF_DONT_IMPLY_DEDENT: i32 = 0x0200;
pub(crate) const PY_CF_ONLY_AST: i32 = 0x0400;
pub(crate) const PY_CF_IGNORE_COOKIE: i32 = 0x0800;
pub(crate) const PY_CF_TYPE_COMMENTS: i32 = 0x1000;
pub(crate) const PY_CF_ALLOW_TOP_LEVEL_AWAIT: i32 = 0x2000;
pub(crate) const PY_CF_ALLOW_INCOMPLETE_INPUT: i32 = 0x4000;
pub(crate) const PY_CF_OPTIMIZED_AST: i32 = 0x8000 | PY_CF_ONLY_AST;

// __future__ flags - sync with Lib/__future__.py and Include/cpython/compile.h.
const CO_NESTED: i32 = 0x0010;
const CO_FUTURE_DIVISION: i32 = 0x20000;
const CO_FUTURE_ABSOLUTE_IMPORT: i32 = 0x40000;
const CO_FUTURE_WITH_STATEMENT: i32 = 0x80000;
const CO_FUTURE_PRINT_FUNCTION: i32 = 0x100000;
const CO_FUTURE_UNICODE_LITERALS: i32 = 0x200000;
const CO_FUTURE_BARRY_AS_BDFL: i32 = 0x400000;
const CO_FUTURE_GENERATOR_STOP: i32 = 0x800000;
const CO_FUTURE_ANNOTATIONS: i32 = 0x1000000;

const PY_CF_MASK: i32 = CO_FUTURE_DIVISION
    | CO_FUTURE_ABSOLUTE_IMPORT
    | CO_FUTURE_WITH_STATEMENT
    | CO_FUTURE_PRINT_FUNCTION
    | CO_FUTURE_UNICODE_LITERALS
    | CO_FUTURE_BARRY_AS_BDFL
    | CO_FUTURE_GENERATOR_STOP
    | CO_FUTURE_ANNOTATIONS;
const PY_CF_MASK_OBSOLETE: i32 = CO_NESTED;
pub(crate) const PY_CF_COMPILE_MASK: i32 = PY_CF_ONLY_AST
    | PY_CF_ALLOW_TOP_LEVEL_AWAIT
    | PY_CF_TYPE_COMMENTS
    | PY_CF_DONT_IMPLY_DEDENT
    | PY_CF_ALLOW_INCOMPLETE_INPUT
    | PY_CF_OPTIMIZED_AST;
pub(crate) const PY_CF_ALLOWED_FLAGS: i32 = PY_CF_MASK | PY_CF_MASK_OBSOLETE | PY_CF_COMPILE_MASK;

pub(crate) fn compile_future_feature_mask() -> bytecode::CodeFlags {
    // RustPython accepts barry_as_FLUFL but leaves its parser mode disabled.
    bytecode::CodeFlags::FUTURE_DIVISION
        | bytecode::CodeFlags::FUTURE_ABSOLUTE_IMPORT
        | bytecode::CodeFlags::FUTURE_WITH_STATEMENT
        | bytecode::CodeFlags::FUTURE_PRINT_FUNCTION
        | bytecode::CodeFlags::FUTURE_UNICODE_LITERALS
        | bytecode::CodeFlags::FUTURE_GENERATOR_STOP
        | bytecode::CodeFlags::FUTURE_ANNOTATIONS
}

pub(crate) fn compile_future_features_from_flags(flags: i32) -> bytecode::CodeFlags {
    bytecode::CodeFlags::from_bits_truncate(flags as u32 & compile_future_feature_mask().bits())
}
