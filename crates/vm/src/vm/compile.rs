//! Python code compilation functions.
//!
//! For code execution functions, see python_run.rs

use core::fmt;

use crate::{
    AsObject, PyObjectRef, PyRef, PyResult, VirtualMachine,
    builtins::{PyBaseExceptionRef, PyCode},
    compiler::{self, CompileError, CompileOpts},
    vm::compile_mode::{
        PY_CF_ALLOW_INCOMPLETE_INPUT, PY_CF_ALLOW_TOP_LEVEL_AWAIT, PY_CF_DONT_IMPLY_DEDENT,
        PY_CF_IGNORE_COOKIE, PY_CF_ONLY_AST, PY_CF_OPTIMIZED_AST, PY_CF_TYPE_COMMENTS,
        PY_EVAL_INPUT, PY_FILE_INPUT, PY_FUNC_TYPE_INPUT, PY_SINGLE_INPUT,
        compile_future_features_from_flags,
    },
};

#[derive(Debug)]
pub enum VmCompileError {
    Compile(CompileError),
    Warning(CompileWarningError),
}

#[derive(Debug)]
pub struct CompileWarningError {
    exception: PyBaseExceptionRef,
    filename: String,
    lineno: usize,
    offset: usize,
}

impl From<CompileError> for VmCompileError {
    fn from(err: CompileError) -> Self {
        Self::Compile(err)
    }
}

impl fmt::Display for VmCompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compile(err) => err.fmt(f),
            Self::Warning(_) => f.write_str("compiler warning raised as an exception"),
        }
    }
}

impl VmCompileError {
    pub fn into_pyexception(self, vm: &VirtualMachine, source: Option<&str>) -> PyBaseExceptionRef {
        match self {
            Self::Compile(err) => vm.new_syntax_error(&err, source),
            Self::Warning(err) => err.into_pyexception(vm, source),
        }
    }

    pub fn into_pyexception_maybe_incomplete(
        self,
        vm: &VirtualMachine,
        source: Option<&str>,
        allow_incomplete: bool,
    ) -> PyBaseExceptionRef {
        match self {
            Self::Compile(err) => {
                vm.new_syntax_error_maybe_incomplete(&err, source, allow_incomplete)
            }
            Self::Warning(err) => err.into_pyexception(vm, source),
        }
    }
}

impl CompileWarningError {
    fn into_codegen_error(
        self,
        location: compiler::core::SourceLocation,
        source_path: String,
        vm: &VirtualMachine,
    ) -> compiler::codegen::error::CodegenError {
        let message = self.exception.as_object().str(vm).map_or_else(
            |_| "compiler warning raised as an exception".to_owned(),
            |message| message.as_wtf8().to_string(),
        );
        compiler::codegen::error::CodegenError {
            location: Some(location),
            error: compiler::codegen::error::CodegenErrorType::SyntaxError(message),
            source_path,
        }
    }

    fn into_pyexception(self, vm: &VirtualMachine, source: Option<&str>) -> PyBaseExceptionRef {
        if !self
            .exception
            .fast_isinstance(vm.ctx.exceptions.syntax_warning)
        {
            return self.exception;
        }
        let Ok(message) = self.exception.as_object().str(vm) else {
            return self.exception;
        };
        let syntax_error = vm.new_exception_msg(
            vm.ctx.exceptions.syntax_error.to_owned(),
            message.as_wtf8().to_owned(),
        );
        syntax_error
            .as_object()
            .set_attr("lineno", vm.ctx.new_int(self.lineno), vm)
            .unwrap();
        syntax_error
            .as_object()
            .set_attr("offset", vm.ctx.new_int(self.offset), vm)
            .unwrap();
        syntax_error
            .as_object()
            .set_attr("filename", vm.ctx.new_str(self.filename), vm)
            .unwrap();
        let text = source
            .and_then(|source| source.split('\n').nth(self.lineno.saturating_sub(1)))
            .map_or_else(
                || vm.ctx.none(),
                |line| {
                    vm.ctx
                        .new_str(format!("{}\n", line.trim_end_matches('\r')))
                        .into()
                },
            );
        syntax_error.as_object().set_attr("text", text, vm).unwrap();
        syntax_error
    }
}

impl VirtualMachine {
    #[cfg(feature = "parser")]
    fn detect_source_encoding(source: &[u8]) -> Option<String> {
        fn find_encoding_in_line(line: &[u8]) -> Option<String> {
            let hash_pos = line.iter().position(|&b| b == b'#')?;
            if !line[..hash_pos]
                .iter()
                .all(|&b| b == b' ' || b == b'\t' || b == b'\x0c' || b == b'\r')
            {
                return None;
            }
            let after_hash = &line[hash_pos..];
            let coding_pos = after_hash.windows(6).position(|w| w == b"coding")?;
            let after_coding = &after_hash[coding_pos + 6..];
            let rest = if after_coding.first() == Some(&b':') || after_coding.first() == Some(&b'=')
            {
                &after_coding[1..]
            } else {
                return None;
            };
            let name: String = rest
                .iter()
                .copied()
                .skip_while(|&b| b == b' ' || b == b'\t')
                .take_while(|&b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
                .map(|b| b as char)
                .collect();
            (!name.is_empty()).then(|| VirtualMachine::normalize_source_encoding(&name))
        }

        let mut lines = source.splitn(3, |&b| b == b'\n');
        if let Some(first) = lines.next() {
            let first = first.strip_prefix(b"\xef\xbb\xbf").unwrap_or(first);
            if let Some(enc) = find_encoding_in_line(first) {
                return Some(enc);
            }
            let trimmed = first
                .iter()
                .skip_while(|&&b| b == b' ' || b == b'\t' || b == b'\x0c' || b == b'\r')
                .copied()
                .collect::<Vec<_>>();
            if !trimmed.is_empty() && trimmed[0] != b'#' {
                return None;
            }
        }
        lines.next().and_then(find_encoding_in_line)
    }

    #[cfg(feature = "parser")]
    fn normalize_source_encoding(name: &str) -> String {
        let mut normalized = String::with_capacity(name.len().min(12));
        for ch in name.chars().take(12) {
            if ch == '_' {
                normalized.push('-');
            } else {
                normalized.push(ch.to_ascii_lowercase());
            }
        }

        if normalized == "utf-8" || normalized.starts_with("utf-8-") {
            "utf-8".to_owned()
        } else if normalized == "latin-1"
            || normalized == "iso-8859-1"
            || normalized == "iso-latin-1"
            || normalized.starts_with("latin-1-")
            || normalized.starts_with("iso-8859-1-")
            || normalized.starts_with("iso-latin-1-")
        {
            "iso-8859-1".to_owned()
        } else {
            name.to_owned()
        }
    }

    #[cfg(feature = "parser")]
    fn is_utf8_encoding(name: &str) -> bool {
        name == "utf-8"
    }

    #[cfg(feature = "parser")]
    pub(crate) fn decode_source_bytes(
        &self,
        source: &[u8],
        filename: &str,
        ignore_cookie: bool,
    ) -> PyResult<String> {
        let has_bom = source.starts_with(b"\xef\xbb\xbf");
        let encoding = if ignore_cookie {
            None
        } else {
            Self::detect_source_encoding(source)
        };
        let is_utf8 = encoding.as_deref().is_none_or(Self::is_utf8_encoding);
        if has_bom && !is_utf8 {
            let enc = encoding.as_deref().unwrap_or("utf-8");
            return Err(self.new_exception_msg(
                self.ctx.exceptions.syntax_error.to_owned(),
                format!("encoding problem: {enc} with BOM").into(),
            ));
        }

        if is_utf8 {
            let src = if has_bom { &source[3..] } else { source };
            match core::str::from_utf8(src) {
                Ok(s) => Ok(s.to_owned()),
                Err(e) => {
                    let bad_byte = src[e.valid_up_to()];
                    let line = src[..e.valid_up_to()]
                        .iter()
                        .filter(|&&b| b == b'\n')
                        .count()
                        + 1;
                    Err(self.new_exception_msg(
                        self.ctx.exceptions.syntax_error.to_owned(),
                        format!(
                            "Non-UTF-8 code starting with '\\x{bad_byte:02x}' \
                             on line {line}, but no encoding declared; \
                             see https://peps.python.org/pep-0263/ for details \
                             ({filename}, line {line})"
                        )
                        .into(),
                    ))
                }
            }
        } else {
            let encoding = encoding.as_deref().unwrap();
            let bytes = self.ctx.new_bytes(source.to_vec());
            let decoded = self
                .state
                .codec_registry
                .decode_text(bytes.into(), encoding, None, self)
                .map_err(|exc| {
                    if exc.fast_isinstance(self.ctx.exceptions.lookup_error) {
                        self.new_exception_msg(
                            self.ctx.exceptions.syntax_error.to_owned(),
                            format!("unknown encoding for '{filename}': {encoding}").into(),
                        )
                    } else {
                        exc
                    }
                })?;
            Ok(decoded.to_string_lossy().into_owned())
        }
    }

    #[cfg(feature = "parser")]
    pub fn compile_string_object_with_flags(
        &self,
        source: &[u8],
        filename: &str,
        start: i32,
        flags: i32,
        feature_version: i32,
        optimize: i32,
    ) -> PyResult<PyObjectRef> {
        use crate::convert::ToPyException;
        use crate::stdlib::_ast;

        let source =
            self.decode_source_bytes(source, filename, (flags & PY_CF_IGNORE_COOKIE) != 0)?;
        let source = source.as_str();
        let optimize = match optimize {
            -1 => self.state.config.settings.optimize,
            0..=2 => optimize as u8,
            _ => return Err(self.new_value_error("compile(): invalid optimize value")),
        };
        let allow_incomplete = (flags & PY_CF_ALLOW_INCOMPLETE_INPUT) != 0;
        let type_comments = (flags & PY_CF_TYPE_COMMENTS) != 0;
        let dont_imply_dedent = (flags & PY_CF_DONT_IMPLY_DEDENT) != 0;
        let is_ast_only = (flags & PY_CF_ONLY_AST) != 0;
        let optimized_ast = (flags & PY_CF_OPTIMIZED_AST) == PY_CF_OPTIMIZED_AST;
        let future_features = compile_future_features_from_flags(flags);
        let explicit_future_annotations =
            future_features.contains(crate::bytecode::CodeFlags::FUTURE_ANNOTATIONS);
        let target_version = if is_ast_only {
            Some(ruff_python_ast::PythonVersion {
                major: 3,
                minor: u8::try_from(feature_version).unwrap_or(crate::version::MINOR as u8),
            })
        } else {
            None
        };

        if is_ast_only {
            if start == PY_FUNC_TYPE_INPUT {
                return _ast::parse_func_type(self, source, optimize, target_version)
                    .map_err(|e| (e, Some(source), allow_incomplete).to_pyexception(self));
            }
            let (parser_mode, interactive) = match start {
                PY_SINGLE_INPUT => (ruff_python_parser::Mode::Module, true),
                PY_FILE_INPUT => (ruff_python_parser::Mode::Module, false),
                PY_EVAL_INPUT => (ruff_python_parser::Mode::Expression, false),
                _ => {
                    return Err(
                        self.new_system_error("Invalid start argument passed to Py_CompileString")
                    );
                }
            };
            let parsed = _ast::parse(
                self,
                source,
                parser_mode,
                optimize,
                target_version,
                type_comments,
                optimized_ast,
                interactive,
                explicit_future_annotations,
                dont_imply_dedent,
            )
            .map_err(|e| (e, Some(source), allow_incomplete).to_pyexception(self))?;
            if start == PY_SINGLE_INPUT {
                return _ast::wrap_interactive(self, parsed);
            }
            return Ok(parsed);
        }

        if type_comments {
            let parser_mode = match start {
                PY_SINGLE_INPUT | PY_FILE_INPUT => ruff_python_parser::Mode::Module,
                PY_EVAL_INPUT => ruff_python_parser::Mode::Expression,
                _ => {
                    return Err(
                        self.new_system_error("Invalid start argument passed to Py_CompileString")
                    );
                }
            };
            let _ = _ast::parse(
                self,
                source,
                parser_mode,
                optimize,
                None,
                type_comments,
                false,
                start == PY_SINGLE_INPUT,
                explicit_future_annotations,
                dont_imply_dedent,
            )
            .map_err(|e| (e, Some(source), allow_incomplete).to_pyexception(self))?;
        }

        let mode = match start {
            PY_SINGLE_INPUT => compiler::Mode::Single,
            PY_FILE_INPUT => compiler::Mode::Exec,
            PY_EVAL_INPUT => compiler::Mode::Eval,
            PY_FUNC_TYPE_INPUT => compiler::Mode::BlockExpr,
            _ => {
                return Err(
                    self.new_system_error("Invalid start argument passed to Py_CompileString")
                );
            }
        };
        let mut opts = self.compile_opts();
        opts.optimize = optimize;
        opts.allow_top_level_await = (flags & PY_CF_ALLOW_TOP_LEVEL_AWAIT) != 0;
        opts.future_features = future_features;
        opts.dont_imply_dedent = dont_imply_dedent;
        let code = self
            .compile_with_opts(source, mode, filename, opts)
            .map_err(|err| {
                err.into_pyexception_maybe_incomplete(self, Some(source), allow_incomplete)
            })?;
        Ok(code.into())
    }

    pub fn compile(
        &self,
        source: &str,
        mode: compiler::Mode,
        source_path: impl Into<String>,
    ) -> Result<PyRef<PyCode>, VmCompileError> {
        self.compile_with_opts(source, mode, source_path, self.compile_opts())
    }

    pub fn compile_with_opts(
        &self,
        source: &str,
        mode: compiler::Mode,
        source_path: impl Into<String>,
        opts: CompileOpts,
    ) -> Result<PyRef<PyCode>, VmCompileError> {
        let source_path = source_path.into();
        #[cfg(feature = "parser")]
        {
            self.emit_tokenizer_syntax_warnings(source, &source_path)
                .map_err(VmCompileError::Warning)?;
            self.emit_string_escape_warnings(source, &source_path)
                .map_err(VmCompileError::Warning)?;
        }
        #[cfg(feature = "parser")]
        let code = {
            let mut syntax_warning_handler = |location, message| {
                escape_warnings::warn_syntax_at_location(&source_path, location, message, self)
                    .map_err(|err| err.into_codegen_error(location, source_path.clone(), self))
            };
            compiler::compile_with_syntax_warning_handler(
                source,
                mode,
                &source_path,
                opts,
                &mut syntax_warning_handler,
            )
        };
        #[cfg(not(feature = "parser"))]
        let code = compiler::compile(source, mode, &source_path, opts);
        let code = code
            .map(|code| PyCode::new_ref_from_bytecode(self, code))
            .map_err(VmCompileError::Compile)?;
        Ok(code)
    }
}

/// Scan source for invalid escape sequences in all string literals and emit
/// SyntaxWarning.
///
/// Corresponds to:
/// - `warn_invalid_escape_sequence()` in `Parser/string_parser.c`
/// - `_PyTokenizer_warn_invalid_escape_sequence()` in `Parser/tokenizer/helpers.c`
#[cfg(feature = "parser")]
mod escape_warnings {
    use super::*;
    use crate::warn;
    use ruff_python_ast::{self as ast, visitor::Visitor};
    #[cfg(test)]
    use ruff_text_size::Ranged;
    use ruff_text_size::TextRange;

    /// Calculate 1-indexed line number at byte offset in source.
    fn line_number_at(source: &str, offset: usize) -> usize {
        source[..offset.min(source.len())]
            .bytes()
            .filter(|&b| b == b'\n')
            .count()
            + 1
    }

    fn line_offset_at(source: &str, offset: usize) -> (usize, usize) {
        let offset = offset.min(source.len());
        let prefix = &source[..offset];
        let lineno = prefix.bytes().filter(|&b| b == b'\n').count() + 1;
        let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
        let column = source[line_start..offset].chars().count() + 1;
        (lineno, column)
    }

    fn compile_warning_error(
        exception: PyBaseExceptionRef,
        source: &str,
        filename: &str,
        offset: usize,
    ) -> CompileWarningError {
        let (lineno, offset) = line_offset_at(source, offset);
        CompileWarningError {
            exception,
            filename: filename.to_owned(),
            lineno,
            offset,
        }
    }

    /// Get content bounds (start, end byte offsets) of a quoted string literal,
    /// excluding prefix characters and quote delimiters.
    fn content_bounds(source: &str, range: TextRange) -> Option<(usize, usize)> {
        let s = range.start().to_usize();
        let e = range.end().to_usize();
        if s >= e || e > source.len() {
            return None;
        }
        let bytes = &source.as_bytes()[s..e];
        // Skip prefix (u, b, r, etc.) to find the first quote character.
        let qi = bytes.iter().position(|&c| c == b'\'' || c == b'"')?;
        let qc = bytes[qi];
        let ql = if bytes.get(qi + 1) == Some(&qc) && bytes.get(qi + 2) == Some(&qc) {
            3
        } else {
            1
        };
        let cs = s + qi + ql;
        let ce = e.checked_sub(ql)?;
        if cs <= ce { Some((cs, ce)) } else { None }
    }

    /// Scan `source[start..end]` for the first invalid escape sequence.
    /// Returns `Some((invalid_char, byte_offset_in_source))` for the first
    /// invalid escape found, or `None` if all escapes are valid.
    ///
    /// When `is_bytes` is true, `\u`, `\U`, and `\N` are treated as invalid
    /// (bytes literals only support byte-oriented escapes).
    ///
    /// Only reports the **first** invalid escape per string literal, matching
    /// `_PyUnicode_DecodeUnicodeEscapeInternal2` which stores only the first
    /// `first_invalid_escape_char`.
    fn first_invalid_escape(
        source: &str,
        start: usize,
        end: usize,
        is_bytes: bool,
    ) -> Option<(char, usize)> {
        let raw = &source[start..end];
        let mut chars = raw.char_indices().peekable();
        while let Some((i, ch)) = chars.next() {
            if ch != '\\' {
                continue;
            }
            let Some((_, next)) = chars.next() else {
                break;
            };
            let valid = match next {
                '\\' | '\'' | '"' | 'a' | 'b' | 'f' | 'n' | 'r' | 't' | 'v' => true,
                '\n' => true,
                '\r' => {
                    if matches!(chars.peek(), Some(&(_, '\n'))) {
                        chars.next();
                    }
                    true
                }
                '0'..='7' => {
                    for _ in 0..2 {
                        if matches!(chars.peek(), Some(&(_, '0'..='7'))) {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    true
                }
                'x' | 'u' | 'U' => {
                    // \u and \U are only valid in string literals, not bytes
                    if is_bytes && next != 'x' {
                        false
                    } else {
                        let count = match next {
                            'x' => 2,
                            'u' => 4,
                            'U' => 8,
                            _ => unreachable!(),
                        };
                        for _ in 0..count {
                            if chars.peek().is_some_and(|&(_, c)| c.is_ascii_hexdigit()) {
                                chars.next();
                            } else {
                                break;
                            }
                        }
                        true
                    }
                }
                'N' => {
                    // \N{name} is only valid in string literals, not bytes
                    if is_bytes {
                        false
                    } else {
                        if matches!(chars.peek(), Some(&(_, '{'))) {
                            chars.next();
                            for (_, c) in chars.by_ref() {
                                if c == '}' {
                                    break;
                                }
                            }
                        }
                        true
                    }
                }
                _ => false,
            };
            if !valid {
                return Some((next, start + i));
            }
        }
        None
    }

    /// Emit `SyntaxWarning` for an invalid escape sequence.
    ///
    /// `warn_invalid_escape_sequence()` in `Parser/string_parser.c`
    fn warn_invalid_escape_sequence(
        source: &str,
        ch: char,
        offset: usize,
        filename: &str,
        vm: &VirtualMachine,
    ) -> Result<(), CompileWarningError> {
        let lineno = line_number_at(source, offset);
        let message = vm.ctx.new_str(format!(
            "\"\\{ch}\" is an invalid escape sequence. \
             Such sequences will not work in the future. \
             Did you mean \"\\\\{ch}\"? A raw string is also an option."
        ));
        let fname = vm.ctx.new_str(filename);
        warn::warn_explicit(
            Some(vm.ctx.exceptions.syntax_warning.to_owned()),
            message.into(),
            fname,
            lineno,
            None,
            vm.ctx.none(),
            None,
            None,
            vm,
        )
        .map_err(|err| compile_warning_error(err, source, filename, offset))
    }

    #[cfg(test)]
    #[derive(Copy, Clone, Eq, PartialEq)]
    enum InferredType {
        Tuple,
        List,
        Dict,
        Set,
        Generator,
        Function,
        Template,
        Str,
        Bytes,
        Int,
        Float,
        Complex,
        Bool,
        NoneType,
        Ellipsis,
        Slice,
    }

    #[cfg(test)]
    impl InferredType {
        fn name(self) -> &'static str {
            match self {
                Self::Tuple => "tuple",
                Self::List => "list",
                Self::Dict => "dict",
                Self::Set => "set",
                Self::Generator => "generator",
                Self::Function => "function",
                Self::Template => "string.templatelib.Template",
                Self::Str => "str",
                Self::Bytes => "bytes",
                Self::Int => "int",
                Self::Float => "float",
                Self::Complex => "complex",
                Self::Bool => "bool",
                Self::NoneType => "NoneType",
                Self::Ellipsis => "ellipsis",
                Self::Slice => "slice",
            }
        }

        fn is_long_subclass(self) -> bool {
            matches!(self, Self::Int | Self::Bool)
        }
    }

    #[cfg(test)]
    fn infer_type(expr: &ast::Expr) -> Option<InferredType> {
        match expr {
            ast::Expr::Tuple(_) => Some(InferredType::Tuple),
            ast::Expr::List(_) | ast::Expr::ListComp(_) => Some(InferredType::List),
            ast::Expr::Dict(_) | ast::Expr::DictComp(_) => Some(InferredType::Dict),
            ast::Expr::Set(_) | ast::Expr::SetComp(_) => Some(InferredType::Set),
            ast::Expr::Generator(_) => Some(InferredType::Generator),
            ast::Expr::Lambda(_) => Some(InferredType::Function),
            ast::Expr::TString(_) => Some(InferredType::Template),
            ast::Expr::FString(_) | ast::Expr::StringLiteral(_) => Some(InferredType::Str),
            ast::Expr::BytesLiteral(_) => Some(InferredType::Bytes),
            ast::Expr::NumberLiteral(number) => match number.value {
                ast::Number::Int(_) => Some(InferredType::Int),
                ast::Number::Float(_) => Some(InferredType::Float),
                ast::Number::Complex { .. } => Some(InferredType::Complex),
            },
            ast::Expr::BooleanLiteral(_) => Some(InferredType::Bool),
            ast::Expr::NoneLiteral(_) => Some(InferredType::NoneType),
            ast::Expr::EllipsisLiteral(_) => Some(InferredType::Ellipsis),
            ast::Expr::Slice(_) => Some(InferredType::Slice),
            _ => None,
        }
    }

    #[cfg(test)]
    fn is_constant_expr(expr: &ast::Expr) -> bool {
        matches!(
            expr,
            ast::Expr::StringLiteral(_)
                | ast::Expr::BytesLiteral(_)
                | ast::Expr::NumberLiteral(_)
                | ast::Expr::BooleanLiteral(_)
                | ast::Expr::NoneLiteral(_)
                | ast::Expr::EllipsisLiteral(_)
        )
    }

    #[cfg(test)]
    fn check_is_arg(expr: &ast::Expr) -> bool {
        if let ast::Expr::Tuple(tuple) = expr {
            return !tuple.elts.iter().all(is_constant_expr);
        }
        if !is_constant_expr(expr) {
            return true;
        }
        matches!(
            expr,
            ast::Expr::NoneLiteral(_)
                | ast::Expr::BooleanLiteral(_)
                | ast::Expr::EllipsisLiteral(_)
        )
    }

    #[cfg(test)]
    fn warn_syntax(
        source: &str,
        filename: &str,
        range: TextRange,
        message: String,
        vm: &VirtualMachine,
    ) -> Result<(), CompileWarningError> {
        warn_syntax_at_offset(source, filename, range.start().to_usize(), message, vm)
    }

    fn warn_syntax_at_offset(
        source: &str,
        filename: &str,
        offset: usize,
        message: String,
        vm: &VirtualMachine,
    ) -> Result<(), CompileWarningError> {
        let lineno = line_number_at(source, offset);
        let fname = vm.ctx.new_str(filename);
        let message = vm.ctx.new_str(message);
        warn::warn_explicit(
            Some(vm.ctx.exceptions.syntax_warning.to_owned()),
            message.into(),
            fname,
            lineno,
            None,
            vm.ctx.none(),
            None,
            None,
            vm,
        )
        .map_err(|err| compile_warning_error(err, source, filename, offset))
    }

    pub(super) fn warn_syntax_at_location(
        filename: &str,
        location: compiler::core::SourceLocation,
        message: String,
        vm: &VirtualMachine,
    ) -> Result<(), CompileWarningError> {
        let fname = vm.ctx.new_str(filename);
        let message = vm.ctx.new_str(message);
        warn::warn_explicit(
            Some(vm.ctx.exceptions.syntax_warning.to_owned()),
            message.into(),
            fname,
            location.line.get(),
            None,
            vm.ctx.none(),
            None,
            None,
            vm,
        )
        .map_err(|exception| CompileWarningError {
            exception,
            filename: filename.to_owned(),
            lineno: location.line.get(),
            offset: location.character_offset.get(),
        })
    }

    fn is_ascii_identifier_char(byte: u8) -> bool {
        byte == b'_' || byte.is_ascii_alphanumeric()
    }

    fn numeric_keyword_suffix(rest: &[u8]) -> bool {
        rest.starts_with(b"and")
            || rest.starts_with(b"else")
            || rest.starts_with(b"for")
            || rest.starts_with(b"if")
            || rest.starts_with(b"in")
            || rest.starts_with(b"is")
            || rest.starts_with(b"or")
            || rest.starts_with(b"not")
    }

    fn consume_decimal_digits(bytes: &[u8], mut index: usize) -> usize {
        while index < bytes.len() {
            match bytes[index] {
                b'0'..=b'9' => index += 1,
                b'_' if bytes
                    .get(index + 1)
                    .is_some_and(|byte| byte.is_ascii_digit()) =>
                {
                    index += 2;
                }
                _ => break,
            }
        }
        index
    }

    fn consume_radix_digits(
        bytes: &[u8],
        mut index: usize,
        is_digit: impl Fn(u8) -> bool,
    ) -> usize {
        while index < bytes.len() {
            if is_digit(bytes[index]) {
                index += 1;
            } else if bytes.get(index) == Some(&b'_')
                && bytes.get(index + 1).is_some_and(|&byte| is_digit(byte))
            {
                index += 2;
            } else {
                break;
            }
        }
        index
    }

    fn number_literal_end(bytes: &[u8], start: usize) -> Option<(&'static str, usize)> {
        if bytes.get(start) == Some(&b'.') {
            if !bytes
                .get(start + 1)
                .is_some_and(|byte| byte.is_ascii_digit())
            {
                return None;
            }
            let mut index = consume_decimal_digits(bytes, start + 1);
            index = consume_exponent(bytes, index);
            if matches!(bytes.get(index), Some(b'j' | b'J')) {
                return Some(("imaginary", index + 1));
            }
            return Some(("decimal", index));
        }

        if !bytes.get(start).is_some_and(|byte| byte.is_ascii_digit()) {
            return None;
        }

        if bytes.get(start) == Some(&b'0') {
            match bytes.get(start + 1) {
                Some(b'x' | b'X') => {
                    let end =
                        consume_radix_digits(bytes, start + 2, |byte| byte.is_ascii_hexdigit());
                    return Some(("hexadecimal", end));
                }
                Some(b'o' | b'O') => {
                    let end =
                        consume_radix_digits(bytes, start + 2, |byte| matches!(byte, b'0'..=b'7'));
                    return Some(("octal", end));
                }
                Some(b'b' | b'B') => {
                    let end =
                        consume_radix_digits(bytes, start + 2, |byte| matches!(byte, b'0' | b'1'));
                    return Some(("binary", end));
                }
                _ => {}
            }
        }

        let mut index = consume_decimal_digits(bytes, start);
        if bytes.get(index) == Some(&b'.') {
            index = consume_decimal_digits(bytes, index + 1);
        }
        index = consume_exponent(bytes, index);
        if matches!(bytes.get(index), Some(b'j' | b'J')) {
            return Some(("imaginary", index + 1));
        }
        Some(("decimal", index))
    }

    fn consume_exponent(bytes: &[u8], index: usize) -> usize {
        if !matches!(bytes.get(index), Some(b'e' | b'E')) {
            return index;
        }
        let mut cursor = index + 1;
        if matches!(bytes.get(cursor), Some(b'+' | b'-')) {
            cursor += 1;
        }
        if bytes.get(cursor).is_some_and(|byte| byte.is_ascii_digit()) {
            consume_decimal_digits(bytes, cursor)
        } else {
            index
        }
    }

    fn skip_quoted_string(bytes: &[u8], mut index: usize) -> usize {
        let quote = bytes[index];
        let triple = bytes.get(index + 1) == Some(&quote) && bytes.get(index + 2) == Some(&quote);
        let quote_len = if triple { 3 } else { 1 };
        index += quote_len;
        while index < bytes.len() {
            if bytes[index] == b'\\' {
                index = (index + 2).min(bytes.len());
            } else if triple
                && bytes.get(index) == Some(&quote)
                && bytes.get(index + 1) == Some(&quote)
                && bytes.get(index + 2) == Some(&quote)
            {
                return index + 3;
            } else if !triple && bytes[index] == quote {
                return index + 1;
            } else {
                index += 1;
            }
        }
        index
    }

    fn emit_numeric_literal_warnings(
        source: &str,
        filename: &str,
        vm: &VirtualMachine,
    ) -> Result<(), CompileWarningError> {
        let bytes = source.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'#' => {
                    while index < bytes.len() && bytes[index] != b'\n' {
                        index += 1;
                    }
                }
                b'\'' | b'"' => {
                    index = skip_quoted_string(bytes, index);
                }
                byte if byte >= 0x80 || byte == b'_' || byte.is_ascii_alphabetic() => {
                    index += 1;
                    while index < bytes.len()
                        && (bytes[index] >= 0x80 || is_ascii_identifier_char(bytes[index]))
                    {
                        index += 1;
                    }
                }
                b'.' | b'0'..=b'9' => {
                    let Some((kind, end)) = number_literal_end(bytes, index) else {
                        index += 1;
                        continue;
                    };
                    if end > index && numeric_keyword_suffix(&bytes[end..]) {
                        warn_syntax_at_offset(
                            source,
                            filename,
                            index,
                            format!("invalid {kind} literal"),
                            vm,
                        )?;
                    }
                    index = end.max(index + 1);
                }
                _ => index += 1,
            }
        }
        Ok(())
    }

    #[cfg(test)]
    #[derive(Copy, Clone)]
    struct ControlFlowInFinallyContext {
        in_finally: bool,
        in_funcdef: bool,
        in_loop: bool,
    }

    #[cfg(test)]
    struct CompilerSyntaxWarningVisitor<'a> {
        source: &'a str,
        filename: &'a str,
        vm: &'a VirtualMachine,
        error: Option<CompileWarningError>,
        cf_finally: Vec<ControlFlowInFinallyContext>,
        future_annotations: bool,
        skip_codegen_warnings: bool,
    }

    #[cfg(test)]
    impl<'a> CompilerSyntaxWarningVisitor<'a> {
        fn record_warning(&mut self, result: Result<(), CompileWarningError>) {
            if self.error.is_none()
                && let Err(err) = result
            {
                self.error = Some(err);
            }
        }

        fn push_cf_context(&mut self, in_finally: bool, in_funcdef: bool, in_loop: bool) {
            self.cf_finally.push(ControlFlowInFinallyContext {
                in_finally,
                in_funcdef,
                in_loop,
            });
        }

        fn before_return(&mut self, range: TextRange) {
            if let Some(ctx) = self.cf_finally.last()
                && ctx.in_finally
                && !ctx.in_funcdef
            {
                let result = warn_syntax(
                    self.source,
                    self.filename,
                    range,
                    "'return' in a 'finally' block".to_owned(),
                    self.vm,
                );
                self.record_warning(result);
            }
        }

        fn before_loop_exit(&mut self, range: TextRange, kw: &str) {
            if let Some(ctx) = self.cf_finally.last()
                && ctx.in_finally
                && !ctx.in_loop
            {
                let result = warn_syntax(
                    self.source,
                    self.filename,
                    range,
                    format!("'{kw}' in a 'finally' block"),
                    self.vm,
                );
                self.record_warning(result);
            }
        }

        fn visit_body_with_context(
            &mut self,
            body: &'a [ast::Stmt],
            in_finally: bool,
            in_funcdef: bool,
            in_loop: bool,
        ) {
            self.push_cf_context(in_finally, in_funcdef, in_loop);
            self.visit_body(body);
            self.cf_finally.pop();
        }

        fn visit_parameter_annotation(&mut self, parameter: &'a ast::Parameter) {
            if !self.future_annotations
                && let Some(annotation) = &parameter.annotation
            {
                self.visit_expr(annotation);
            }
        }

        fn visit_positional_defaults(&mut self, parameters: &'a ast::Parameters) {
            for parameter in parameters.posonlyargs.iter().chain(&parameters.args) {
                if let Some(default) = &parameter.default {
                    self.visit_expr(default);
                }
            }
        }

        fn visit_kwonly_defaults(&mut self, parameters: &'a ast::Parameters) {
            for parameter in &parameters.kwonlyargs {
                if let Some(default) = &parameter.default {
                    self.visit_expr(default);
                }
            }
        }

        fn visit_function_annotations_cpython_order(
            &mut self,
            parameters: &'a ast::Parameters,
            returns: Option<&'a ast::Expr>,
        ) {
            if self.future_annotations {
                return;
            }
            for parameter in &parameters.args {
                self.visit_parameter_annotation(&parameter.parameter);
            }
            for parameter in &parameters.posonlyargs {
                self.visit_parameter_annotation(&parameter.parameter);
            }
            if let Some(vararg) = &parameters.vararg {
                self.visit_parameter_annotation(vararg);
            }
            for parameter in &parameters.kwonlyargs {
                self.visit_parameter_annotation(&parameter.parameter);
            }
            if let Some(kwarg) = &parameters.kwarg {
                self.visit_parameter_annotation(kwarg);
            }
            if let Some(returns) = returns {
                self.visit_annotation(returns);
            }
        }

        fn check_compare(&mut self, compare: &'a ast::ExprCompare) {
            let mut left = check_is_arg(&compare.left);
            let mut left_expr = compare.left.as_ref();
            for (op, right_expr) in compare.ops.iter().zip(compare.comparators.iter()) {
                if self.error.is_some() {
                    return;
                }
                let right = check_is_arg(right_expr);
                if matches!(op, ast::CmpOp::Is | ast::CmpOp::IsNot) && (!right || !left) {
                    let literal = if !left { left_expr } else { right_expr };
                    if let Some(inferred) = infer_type(literal) {
                        let is_op = matches!(op, ast::CmpOp::Is);
                        let op = if is_op { "\"is\"" } else { "\"is not\"" };
                        let replacement = if is_op { "==" } else { "!=" };
                        let result = warn_syntax(
                            self.source,
                            self.filename,
                            compare.range,
                            format!(
                                "{op} with '{}' literal. Did you mean \"{replacement}\"?",
                                inferred.name()
                            ),
                            self.vm,
                        );
                        self.record_warning(result);
                        return;
                    }
                }
                left = right;
                left_expr = right_expr;
            }
        }

        fn check_caller(&mut self, func: &'a ast::Expr) {
            if matches!(
                func,
                ast::Expr::StringLiteral(_)
                    | ast::Expr::BytesLiteral(_)
                    | ast::Expr::NumberLiteral(_)
                    | ast::Expr::BooleanLiteral(_)
                    | ast::Expr::NoneLiteral(_)
                    | ast::Expr::EllipsisLiteral(_)
                    | ast::Expr::Tuple(_)
                    | ast::Expr::List(_)
                    | ast::Expr::ListComp(_)
                    | ast::Expr::Dict(_)
                    | ast::Expr::DictComp(_)
                    | ast::Expr::Set(_)
                    | ast::Expr::SetComp(_)
                    | ast::Expr::Generator(_)
                    | ast::Expr::FString(_)
                    | ast::Expr::TString(_)
            ) && let Some(inferred) = infer_type(func)
            {
                let result = warn_syntax(
                    self.source,
                    self.filename,
                    func.range(),
                    format!(
                        "'{}' object is not callable; perhaps you missed a comma?",
                        inferred.name()
                    ),
                    self.vm,
                );
                self.record_warning(result);
            }
        }

        fn check_subscripter(&mut self, value: &'a ast::Expr) {
            let warns = matches!(
                value,
                ast::Expr::NoneLiteral(_)
                    | ast::Expr::EllipsisLiteral(_)
                    | ast::Expr::NumberLiteral(_)
                    | ast::Expr::BooleanLiteral(_)
                    | ast::Expr::Set(_)
                    | ast::Expr::SetComp(_)
                    | ast::Expr::Generator(_)
                    | ast::Expr::TString(_)
                    | ast::Expr::Lambda(_)
            );
            if warns && let Some(inferred) = infer_type(value) {
                let result = warn_syntax(
                    self.source,
                    self.filename,
                    value.range(),
                    format!(
                        "'{}' object is not subscriptable; perhaps you missed a comma?",
                        inferred.name()
                    ),
                    self.vm,
                );
                self.record_warning(result);
            }
        }

        fn check_index(&mut self, value: &'a ast::Expr, slice: &'a ast::Expr) {
            let Some(index_type) = infer_type(slice) else {
                return;
            };
            if index_type.is_long_subclass() || index_type == InferredType::Slice {
                return;
            }

            let warns = matches!(
                value,
                ast::Expr::StringLiteral(_)
                    | ast::Expr::BytesLiteral(_)
                    | ast::Expr::Tuple(_)
                    | ast::Expr::List(_)
                    | ast::Expr::ListComp(_)
                    | ast::Expr::FString(_)
            );
            if warns && let Some(value_type) = infer_type(value) {
                let result = warn_syntax(
                    self.source,
                    self.filename,
                    value.range(),
                    format!(
                        "{} indices must be integers or slices, not {}; perhaps you missed a comma?",
                        value_type.name(),
                        index_type.name()
                    ),
                    self.vm,
                );
                self.record_warning(result);
            }
        }

        fn check_assert(&mut self, assert_stmt: &'a ast::StmtAssert) {
            if matches!(&*assert_stmt.test, ast::Expr::Tuple(tuple) if !tuple.elts.is_empty()) {
                let result = warn_syntax(
                    self.source,
                    self.filename,
                    assert_stmt.range,
                    "assertion is always true, perhaps remove parentheses?".to_owned(),
                    self.vm,
                );
                self.record_warning(result);
            }
        }
    }

    #[cfg(test)]
    impl<'a> Visitor<'a> for CompilerSyntaxWarningVisitor<'a> {
        fn visit_stmt(&mut self, stmt: &'a ast::Stmt) {
            if self.error.is_some() {
                return;
            }

            match stmt {
                ast::Stmt::FunctionDef(function) => {
                    for decorator in &function.decorator_list {
                        self.visit_decorator(decorator);
                    }
                    self.visit_positional_defaults(&function.parameters);
                    self.visit_kwonly_defaults(&function.parameters);
                    if let Some(type_params) = &function.type_params {
                        self.visit_type_params(type_params);
                    }
                    self.visit_function_annotations_cpython_order(
                        &function.parameters,
                        function.returns.as_deref(),
                    );
                    self.visit_body_with_context(&function.body, false, true, false);
                }
                ast::Stmt::ClassDef(class) => {
                    for decorator in &class.decorator_list {
                        self.visit_decorator(decorator);
                    }
                    if let Some(type_params) = &class.type_params {
                        self.visit_type_params(type_params);
                    }
                    self.visit_body(&class.body);
                    if let Some(arguments) = &class.arguments {
                        for base in &arguments.args {
                            self.visit_expr(base);
                        }
                        for keyword in &arguments.keywords {
                            self.visit_keyword(keyword);
                        }
                    }
                }
                ast::Stmt::TypeAlias(type_alias) => {
                    if let Some(type_params) = &type_alias.type_params {
                        self.visit_type_params(type_params);
                    }
                    self.visit_expr(&type_alias.value);
                }
                ast::Stmt::Return(return_stmt) => {
                    self.before_return(return_stmt.range);
                    if let Some(value) = &return_stmt.value {
                        self.visit_expr(value);
                    }
                }
                ast::Stmt::AnnAssign(ann_assign) => {
                    self.visit_expr(&ann_assign.target);
                    if !self.future_annotations {
                        self.visit_annotation(&ann_assign.annotation);
                    }
                    if let Some(value) = &ann_assign.value {
                        self.visit_expr(value);
                    }
                }
                ast::Stmt::For(for_stmt) => {
                    self.visit_expr(&for_stmt.target);
                    self.visit_expr(&for_stmt.iter);
                    self.visit_body_with_context(&for_stmt.body, false, false, true);
                    self.visit_body(&for_stmt.orelse);
                }
                ast::Stmt::While(while_stmt) => {
                    self.visit_expr(&while_stmt.test);
                    self.visit_body_with_context(&while_stmt.body, false, false, true);
                    self.visit_body(&while_stmt.orelse);
                }
                ast::Stmt::Try(try_stmt) => {
                    self.visit_body(&try_stmt.body);
                    for handler in &try_stmt.handlers {
                        self.visit_except_handler(handler);
                    }
                    self.visit_body(&try_stmt.orelse);
                    self.visit_body_with_context(&try_stmt.finalbody, true, false, false);
                }
                ast::Stmt::Assert(assert_stmt) => {
                    if !self.skip_codegen_warnings {
                        self.check_assert(assert_stmt);
                    }
                    self.visit_expr(&assert_stmt.test);
                    if let Some(msg) = &assert_stmt.msg {
                        self.visit_expr(msg);
                    }
                }
                ast::Stmt::Break(break_stmt) => {
                    self.before_loop_exit(break_stmt.range, "break");
                }
                ast::Stmt::Continue(continue_stmt) => {
                    self.before_loop_exit(continue_stmt.range, "continue");
                }
                _ => ast::visitor::walk_stmt(self, stmt),
            }
        }

        fn visit_expr(&mut self, expr: &'a ast::Expr) {
            if self.error.is_some() {
                return;
            }
            match expr {
                ast::Expr::Compare(compare) if !self.skip_codegen_warnings => {
                    self.check_compare(compare);
                }
                ast::Expr::Call(call) if !self.skip_codegen_warnings => {
                    self.check_caller(&call.func);
                }
                ast::Expr::Subscript(subscript)
                    if !self.skip_codegen_warnings
                        && matches!(subscript.ctx, ast::ExprContext::Load) =>
                {
                    self.check_subscripter(&subscript.value);
                    self.check_index(&subscript.value, &subscript.slice);
                }
                _ => {}
            }
            ast::visitor::walk_expr(self, expr);
        }
    }

    #[cfg(test)]
    fn has_future_annotations(ast: &ast::Mod) -> bool {
        let ast::Mod::Module(module) = ast else {
            return false;
        };
        let mut statements = module.body.iter();
        if let Some(ast::Stmt::Expr(ast::StmtExpr { value, .. })) = statements.clone().next()
            && matches!(&**value, ast::Expr::StringLiteral(_))
        {
            statements.next();
        }
        for statement in statements {
            match statement {
                ast::Stmt::ImportFrom(ast::StmtImportFrom {
                    module,
                    names,
                    level,
                    ..
                }) if *level == 0
                    && module.as_ref().map(|id| id.as_str()) == Some("__future__") =>
                {
                    if names
                        .iter()
                        .any(|future| future.name.as_str() == "annotations")
                    {
                        return true;
                    }
                }
                _ => return false,
            }
        }
        false
    }

    struct EscapeWarningVisitor<'a> {
        source: &'a str,
        filename: &'a str,
        vm: &'a VirtualMachine,
        error: Option<CompileWarningError>,
    }

    impl<'a> EscapeWarningVisitor<'a> {
        fn record_warning(&mut self, result: Result<(), CompileWarningError>) {
            if self.error.is_none()
                && let Err(err) = result
            {
                self.error = Some(err);
            }
        }

        /// Check a quoted string/bytes literal for invalid escapes.
        /// The range must include the prefix and quote delimiters.
        fn check_quoted_literal(&mut self, range: TextRange, is_bytes: bool) {
            if let Some((start, end)) = content_bounds(self.source, range)
                && let Some((ch, offset)) = first_invalid_escape(self.source, start, end, is_bytes)
            {
                let result =
                    warn_invalid_escape_sequence(self.source, ch, offset, self.filename, self.vm);
                self.record_warning(result);
            }
        }

        /// Check an f-string literal element for invalid escapes.
        /// The range covers content only (no prefix/quotes).
        ///
        /// Also handles `\{` / `\}` at the literal–interpolation boundary,
        /// equivalent to `_PyTokenizer_warn_invalid_escape_sequence` handling
        /// `FSTRING_MIDDLE` / `FSTRING_END` tokens.
        fn check_fstring_literal(&mut self, range: TextRange) {
            let start = range.start().to_usize();
            let end = range.end().to_usize();
            if start >= end || end > self.source.len() {
                return;
            }
            if let Some((ch, offset)) = first_invalid_escape(self.source, start, end, false) {
                let result =
                    warn_invalid_escape_sequence(self.source, ch, offset, self.filename, self.vm);
                self.record_warning(result);
                return;
            }
            // In CPython, _PyTokenizer_warn_invalid_escape_sequence handles
            // `\{` and `\}` for FSTRING_MIDDLE/FSTRING_END tokens.  Ruff
            // splits the literal element before the interpolation delimiter,
            // so the `\` sits at the end of the literal range and the `{`/`}`
            // sits just after it.  Only warn when the number of trailing
            // backslashes is odd (an even count means they are all escaped).
            let trailing_bs = self.source.as_bytes()[start..end]
                .iter()
                .rev()
                .take_while(|&&b| b == b'\\')
                .count();
            if trailing_bs % 2 == 1
                && let Some(&after) = self.source.as_bytes().get(end)
                && (after == b'{' || after == b'}')
            {
                let result = warn_invalid_escape_sequence(
                    self.source,
                    after as char,
                    end - 1,
                    self.filename,
                    self.vm,
                );
                self.record_warning(result);
            }
        }

        /// Visit f-string elements, checking literals and recursing into
        /// interpolation expressions and format specs.
        fn visit_fstring_elements(&mut self, elements: &'a ast::InterpolatedStringElements) {
            for element in elements {
                if self.error.is_some() {
                    return;
                }
                match element {
                    ast::InterpolatedStringElement::Literal(lit) => {
                        self.check_fstring_literal(lit.range);
                    }
                    ast::InterpolatedStringElement::Interpolation(interp) => {
                        self.visit_expr(&interp.expression);
                        if let Some(spec) = &interp.format_spec {
                            self.visit_fstring_elements(&spec.elements);
                        }
                    }
                }
            }
        }
    }

    impl<'a> Visitor<'a> for EscapeWarningVisitor<'a> {
        fn visit_expr(&mut self, expr: &'a ast::Expr) {
            if self.error.is_some() {
                return;
            }
            match expr {
                // Regular string literals — decode_unicode_with_escapes path
                ast::Expr::StringLiteral(string) => {
                    for part in string.value.as_slice() {
                        if !matches!(
                            part.flags.prefix(),
                            ast::str_prefix::StringLiteralPrefix::Raw { .. }
                        ) {
                            self.check_quoted_literal(part.range, false);
                        }
                    }
                }
                // Byte string literals — decode_bytes_with_escapes path
                ast::Expr::BytesLiteral(bytes) => {
                    for part in bytes.value.as_slice() {
                        if !matches!(
                            part.flags.prefix(),
                            ast::str_prefix::ByteStringPrefix::Raw { .. }
                        ) {
                            self.check_quoted_literal(part.range, true);
                        }
                    }
                }
                // F-string literals — tokenizer + string_parser paths
                ast::Expr::FString(fstring_expr) => {
                    for part in fstring_expr.value.as_slice() {
                        match part {
                            ast::FStringPart::Literal(string_lit) => {
                                // Plain string part in f-string concatenation
                                if !matches!(
                                    string_lit.flags.prefix(),
                                    ast::str_prefix::StringLiteralPrefix::Raw { .. }
                                ) {
                                    self.check_quoted_literal(string_lit.range, false);
                                }
                            }
                            ast::FStringPart::FString(fstring) => {
                                if matches!(
                                    fstring.flags.prefix(),
                                    ast::str_prefix::FStringPrefix::Raw { .. }
                                ) {
                                    continue;
                                }
                                self.visit_fstring_elements(&fstring.elements);
                            }
                        }
                    }
                }
                _ => ast::visitor::walk_expr(self, expr),
            }
        }
    }

    impl VirtualMachine {
        /// Emit tokenizer-level SyntaxWarnings that CPython raises before
        /// code generation.
        pub(super) fn emit_tokenizer_syntax_warnings(
            &self,
            source: &str,
            filename: &str,
        ) -> Result<(), CompileWarningError> {
            emit_numeric_literal_warnings(source, filename, self)
        }

        /// Walk all string literals in `source` and emit `SyntaxWarning` for
        /// each that contains an invalid escape sequence.
        pub(super) fn emit_string_escape_warnings(
            &self,
            source: &str,
            filename: &str,
        ) -> Result<(), CompileWarningError> {
            let Ok(parsed) =
                ruff_python_parser::parse(source, ruff_python_parser::Mode::Module.into())
            else {
                return Ok(());
            };
            let ast = parsed.into_syntax();
            let mut visitor = EscapeWarningVisitor {
                source,
                filename,
                vm: self,
                error: None,
            };
            match &ast {
                ast::Mod::Module(module) => {
                    for stmt in &module.body {
                        visitor.visit_stmt(stmt);
                    }
                }
                ast::Mod::Expression(expr) => {
                    visitor.visit_expr(&expr.body);
                }
            }
            visitor.error.map_or(Ok(()), Err)
        }

        /// Emit CPython codegen SyntaxWarnings for suspicious compare, call,
        /// and subscript forms.
        #[cfg(test)]
        pub(super) fn emit_compiler_syntax_warnings(
            &self,
            source: &str,
            filename: &str,
        ) -> Result<(), CompileWarningError> {
            self.emit_compiler_syntax_warnings_with_options(source, filename, false, false)
        }

        #[cfg(test)]
        fn emit_compiler_syntax_warnings_with_options(
            &self,
            source: &str,
            filename: &str,
            skip_codegen_warnings: bool,
            explicit_future_annotations: bool,
        ) -> Result<(), CompileWarningError> {
            let Ok(parsed) =
                ruff_python_parser::parse(source, ruff_python_parser::Mode::Module.into())
            else {
                return Ok(());
            };
            let ast = parsed.into_syntax();
            self.emit_compiler_syntax_warnings_from_ast(
                &ast,
                source,
                filename,
                skip_codegen_warnings,
                explicit_future_annotations,
            )
        }

        #[cfg(test)]
        fn emit_compiler_syntax_warnings_from_ast(
            &self,
            ast: &ast::Mod,
            source: &str,
            filename: &str,
            skip_codegen_warnings: bool,
            explicit_future_annotations: bool,
        ) -> Result<(), CompileWarningError> {
            let mut visitor = CompilerSyntaxWarningVisitor {
                source,
                filename,
                vm: self,
                error: None,
                cf_finally: Vec::new(),
                future_annotations: explicit_future_annotations || has_future_annotations(ast),
                skip_codegen_warnings,
            };
            match ast {
                ast::Mod::Module(module) => {
                    for stmt in &module.body {
                        visitor.visit_stmt(stmt);
                    }
                }
                ast::Mod::Expression(expr) => {
                    visitor.visit_expr(&expr.body);
                }
            }
            visitor.error.map_or(Ok(()), Err)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::{Interpreter, builtins::PyTuple};

        fn install_syntax_warning_error_filter(vm: &VirtualMachine) {
            let error_filter = PyTuple::new_ref(
                vec![
                    vm.ctx.new_str("error").into(),
                    vm.ctx.none(),
                    vm.ctx.exceptions.syntax_warning.as_object().to_owned(),
                    vm.ctx.none(),
                    vm.ctx.new_int(0).into(),
                ],
                &vm.ctx,
            );
            vm.state
                .warnings
                .filters
                .borrow_vec_mut()
                .insert(0, error_filter.into());
            vm.state.warnings.filters_mutated();
        }

        fn first_compiler_warning(source: &str) -> String {
            Interpreter::without_stdlib(Default::default()).enter(|vm| {
                install_syntax_warning_error_filter(vm);
                let err = vm
                    .emit_compiler_syntax_warnings(source, "<test>")
                    .expect_err("expected compiler SyntaxWarning");
                err.exception
                    .as_object()
                    .str(vm)
                    .expect("warning message should stringify")
                    .as_wtf8()
                    .to_string()
            })
        }

        fn compile_error_message(source: &str) -> String {
            Interpreter::without_stdlib(Default::default()).enter(|vm| {
                install_syntax_warning_error_filter(vm);
                let err = match vm.compile(source, compiler::Mode::Exec, "<test>") {
                    Ok(_) => panic!("expected compile error"),
                    Err(err) => err,
                };
                err.into_pyexception(vm, Some(source))
                    .as_object()
                    .str(vm)
                    .expect("compile error should stringify")
                    .as_wtf8()
                    .to_string()
            })
        }

        #[test]
        fn codegen_caller_warning_precedes_later_return_error() {
            let message = compile_error_message("(1)()\nreturn\n");
            assert!(
                message.contains("'int' object is not callable"),
                "expected caller SyntaxWarning first, got {message:?}"
            );
        }

        #[test]
        fn symboltable_error_still_precedes_codegen_caller_warning() {
            let message = compile_error_message("(1)()\ndef f():\n    from x import *\n");
            assert!(
                message.contains("import * only allowed at module level"),
                "expected symboltable error first, got {message:?}"
            );
        }

        #[test]
        fn codegen_compare_warning_precedes_later_return_error() {
            let message = compile_error_message("1 is 1\nreturn\n");
            assert!(
                message.contains("\"is\" with 'int' literal"),
                "expected compare SyntaxWarning first, got {message:?}"
            );
        }

        #[test]
        fn codegen_assert_warning_precedes_later_return_error() {
            let message = compile_error_message("assert (1,)\nreturn\n");
            assert!(
                message.contains("assertion is always true"),
                "expected assert SyntaxWarning first, got {message:?}"
            );
        }

        #[test]
        fn codegen_subscript_warning_precedes_later_return_error() {
            let message = compile_error_message("(1)[None]\nreturn\n");
            assert!(
                message.contains("'int' object is not subscriptable"),
                "expected subscript SyntaxWarning first, got {message:?}"
            );
        }

        #[test]
        fn codegen_index_warning_precedes_later_return_error() {
            let message = compile_error_message("'x'[None]\nreturn\n");
            assert!(
                message.contains("str indices must be integers or slices, not NoneType"),
                "expected index SyntaxWarning first, got {message:?}"
            );
        }

        #[test]
        fn string_escape_warning_precedes_later_return_error() {
            let message = compile_error_message("\"\\z\"\nreturn\n");
            assert!(
                message.contains("\"\\z\" is an invalid escape sequence"),
                "expected invalid escape SyntaxWarning first, got {message:?}"
            );
        }

        #[test]
        fn string_escape_warning_precedes_later_symboltable_error() {
            let message = compile_error_message("\"\\z\"\ndef f():\n    from x import *\n");
            assert!(
                message.contains("\"\\z\" is an invalid escape sequence"),
                "expected invalid escape SyntaxWarning first, got {message:?}"
            );
        }

        #[test]
        fn ast_preprocess_finally_warning_precedes_later_return_error() {
            let message = compile_error_message("try:\n    pass\nfinally:\n    return\nreturn\n");
            assert!(
                message.contains("'return' in a 'finally' block"),
                "expected finally SyntaxWarning first, got {message:?}"
            );
        }

        #[test]
        fn ast_preprocess_finally_warning_precedes_symboltable_error() {
            let message = compile_error_message(
                "def f():\n    from x import *\ntry:\n    pass\nfinally:\n    return\n",
            );
            assert!(
                message.contains("'return' in a 'finally' block"),
                "expected finally SyntaxWarning first, got {message:?}"
            );
        }

        #[test]
        fn compiler_warning_visits_function_decorators_before_defaults_and_body() {
            let message = first_compiler_warning(
                r#"
@(b"decorator")()
def f(x=(1)()):
    assert (1,)
"#,
            );
            assert!(
                message.contains("'bytes' object is not callable"),
                "expected decorator warning first, got {message:?}"
            );
        }

        #[test]
        fn compiler_warning_visits_function_defaults_before_annotations() {
            let message = first_compiler_warning(
                r#"
def f(x: (1)() = ("default")()):
    pass
"#,
            );
            assert!(
                message.contains("'str' object is not callable"),
                "expected default warning before annotation warning, got {message:?}"
            );
        }

        #[test]
        fn compiler_warning_visits_class_decorators_before_body_and_bases() {
            let message = first_compiler_warning(
                r#"
@(b"decorator")()
class C((1)()):
    assert (1,)
"#,
            );
            assert!(
                message.contains("'bytes' object is not callable"),
                "expected class decorator warning first, got {message:?}"
            );
        }

        #[test]
        fn compiler_warning_visits_class_body_before_bases() {
            let message = first_compiler_warning(
                r#"
class C((1)()):
    assert (1,)
"#,
            );
            assert!(
                message.contains("assertion is always true"),
                "expected class body warning before base warning, got {message:?}"
            );
        }

        #[test]
        fn compiler_warning_visits_type_alias_type_params_before_value() {
            let message = first_compiler_warning(
                r#"
type Alias[T: (1)()] = ("value")()
"#,
            );
            assert!(
                message.contains("'int' object is not callable"),
                "expected type parameter warning before alias value warning, got {message:?}"
            );
        }
    }
}
