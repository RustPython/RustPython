use crate::bytecode;

pub(crate) const PY_SINGLE_INPUT: i32 = 256;
pub(crate) const PY_FILE_INPUT: i32 = 257;
pub(crate) const PY_EVAL_INPUT: i32 = 258;
pub(crate) const PY_FUNC_TYPE_INPUT: i32 = 345;

bitflags::bitflags! {
    /// `PyCF_*` compiler flags together with the `__future__` `CO_FUTURE_*`
    /// bits, mirroring `PyCompilerFlags.cf_flags`.
    ///
    /// Caveat emptor: these flags are undocumented on purpose and depending on
    /// their effect outside the standard library is **unsupported**.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub(crate) struct CompilerFlags: i32 {
        const SOURCE_IS_UTF8 = 0x0100;
        const DONT_IMPLY_DEDENT = 0x0200;
        const ONLY_AST = 0x0400;
        const IGNORE_COOKIE = 0x0800;
        const TYPE_COMMENTS = 0x1000;
        const ALLOW_TOP_LEVEL_AWAIT = 0x2000;
        const ALLOW_INCOMPLETE_INPUT = 0x4000;
        const OPTIMIZED_AST = 0x8000 | Self::ONLY_AST.bits();

        // __future__ flags - sync with Lib/__future__.py and Include/cpython/compile.h.
        const NESTED = 0x0010;
        const FUTURE_DIVISION = 0x20000;
        const FUTURE_ABSOLUTE_IMPORT = 0x40000;
        const FUTURE_WITH_STATEMENT = 0x80000;
        const FUTURE_PRINT_FUNCTION = 0x100000;
        const FUTURE_UNICODE_LITERALS = 0x200000;
        const FUTURE_BARRY_AS_BDFL = 0x400000;
        const FUTURE_GENERATOR_STOP = 0x800000;
        const FUTURE_ANNOTATIONS = 0x1000000;
    }
}

impl CompilerFlags {
    const FUTURE_MASK: Self = Self::FUTURE_DIVISION
        .union(Self::FUTURE_ABSOLUTE_IMPORT)
        .union(Self::FUTURE_WITH_STATEMENT)
        .union(Self::FUTURE_PRINT_FUNCTION)
        .union(Self::FUTURE_UNICODE_LITERALS)
        .union(Self::FUTURE_BARRY_AS_BDFL)
        .union(Self::FUTURE_GENERATOR_STOP)
        .union(Self::FUTURE_ANNOTATIONS);
    const MASK_OBSOLETE: Self = Self::NESTED;
    const COMPILE_MASK: Self = Self::ONLY_AST
        .union(Self::ALLOW_TOP_LEVEL_AWAIT)
        .union(Self::TYPE_COMMENTS)
        .union(Self::DONT_IMPLY_DEDENT)
        .union(Self::ALLOW_INCOMPLETE_INPUT)
        .union(Self::OPTIMIZED_AST);
    pub(crate) const ALLOWED_FLAGS: Self = Self::FUTURE_MASK
        .union(Self::MASK_OBSOLETE)
        .union(Self::COMPILE_MASK);
}

// Python-visible `ast.PyCF_*` attribute values. The flags cross the
// `compile()` boundary as a plain `int`, so the exposed surface stays `i32`.
pub(crate) const PY_CF_SOURCE_IS_UTF8: i32 = CompilerFlags::SOURCE_IS_UTF8.bits();
pub(crate) const PY_CF_DONT_IMPLY_DEDENT: i32 = CompilerFlags::DONT_IMPLY_DEDENT.bits();
pub(crate) const PY_CF_ONLY_AST: i32 = CompilerFlags::ONLY_AST.bits();
pub(crate) const PY_CF_IGNORE_COOKIE: i32 = CompilerFlags::IGNORE_COOKIE.bits();
pub(crate) const PY_CF_TYPE_COMMENTS: i32 = CompilerFlags::TYPE_COMMENTS.bits();
pub(crate) const PY_CF_ALLOW_TOP_LEVEL_AWAIT: i32 = CompilerFlags::ALLOW_TOP_LEVEL_AWAIT.bits();
pub(crate) const PY_CF_ALLOW_INCOMPLETE_INPUT: i32 = CompilerFlags::ALLOW_INCOMPLETE_INPUT.bits();
pub(crate) const PY_CF_OPTIMIZED_AST: i32 = CompilerFlags::OPTIMIZED_AST.bits();

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
