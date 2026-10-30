//! Python code compilation functions.
//!
//! For code execution functions, see python_run.rs

use core::fmt;

use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyRef, PyResult, VirtualMachine,
    builtins::{PyBaseExceptionRef, PyCode, PyStr, PyStrRef},
    compiler::{self, CompileError, CompileOpts},
    vm::compile_mode::{CompileStart, CompilerFlags, compile_future_features_from_flags},
};

/// Warnings already emitted while reading one interactive statement.
///
/// The source is warned about again after each line is read, but the
/// tokenizer warns about each token only once.
#[cfg(feature = "parser")]
#[derive(Default)]
pub(super) struct InteractiveWarnings {
    emitted: alloc::collections::BTreeSet<(usize, String)>,
}

#[derive(Debug)]
pub enum VmCompileError {
    Compile(CompileError),
    Warning(CompileWarningError),
}

#[derive(Debug)]
pub struct CompileWarningError {
    exception: PyBaseExceptionRef,
    filename: String,
    original_filename: Option<PyStrRef>,
    lineno: usize,
    offset: usize,
    replacement: Option<SyntaxErrorReplacement>,
    diagnostic_text: Option<String>,
    // Grammar-action warnings carry no source-text object.
    omit_source_text: bool,
}

#[derive(Debug)]
struct SyntaxErrorReplacement {
    message: String,
    end_offset: usize,
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
        self.into_pyexception_maybe_incomplete(vm, source, false)
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
    fn into_pyexception(self, vm: &VirtualMachine, source: Option<&str>) -> PyBaseExceptionRef {
        if !self
            .exception
            .fast_isinstance(vm.ctx.exceptions.syntax_warning)
        {
            return self.exception;
        }
        let (message, end_offset) = if let Some(replacement) = self.replacement {
            (replacement.message, Some(replacement.end_offset))
        } else {
            let Ok(message) = self.exception.as_object().str(vm) else {
                return self.exception;
            };
            (message.to_string_lossy().into_owned(), None)
        };
        let syntax_error =
            vm.new_exception_msg(vm.ctx.exceptions.syntax_error.to_owned(), message.into());
        syntax_error
            .as_object()
            .set_attr("lineno", vm.ctx.new_int(self.lineno), vm)
            .unwrap();
        syntax_error
            .as_object()
            .set_attr("offset", vm.ctx.new_int(self.offset), vm)
            .unwrap();
        if let Some(end_offset) = end_offset {
            syntax_error
                .as_object()
                .set_attr("end_lineno", vm.ctx.new_int(self.lineno), vm)
                .unwrap();
            syntax_error
                .as_object()
                .set_attr("end_offset", vm.ctx.new_int(end_offset), vm)
                .unwrap();
        }
        syntax_error
            .as_object()
            .set_attr(
                "filename",
                self.original_filename
                    .unwrap_or_else(|| vm.ctx.new_str(self.filename)),
                vm,
            )
            .unwrap();
        let text = if self.omit_source_text {
            vm.ctx.none()
        } else {
            self.diagnostic_text.map_or_else(
                || {
                    source
                        .and_then(|source| source.split('\n').nth(self.lineno.saturating_sub(1)))
                        .map_or_else(
                            || vm.ctx.none(),
                            |line| {
                                vm.ctx
                                    .new_str(format!("{}\n", line.trim_end_matches('\r')))
                                    .into()
                            },
                        )
                },
                |text| vm.ctx.new_str(text).into(),
            )
        };
        syntax_error.as_object().set_attr("text", text, vm).unwrap();
        syntax_error
    }
}

#[cfg(feature = "parser")]
struct SourceEncoding {
    name: String,
    lineno: usize,
    line: core::ops::Range<usize>,
}

impl VirtualMachine {
    #[cfg(feature = "parser")]
    fn detect_source_encoding(source: &[u8]) -> Option<SourceEncoding> {
        fn find_encoding_in_line(line: &[u8]) -> Option<String> {
            let hash_pos = line.iter().position(|&b| b == b'#')?;
            if !line[..hash_pos]
                .iter()
                .all(|&b| b == b' ' || b == b'\t' || b == b'\x0c' || b == b'\r')
            {
                return None;
            }
            let after_hash = &line[hash_pos..];
            for (coding_pos, candidate) in after_hash.windows(6).enumerate() {
                if candidate != b"coding" {
                    continue;
                }
                let after_coding = &after_hash[coding_pos + 6..];
                if !matches!(after_coding.first(), Some(b':' | b'=')) {
                    continue;
                }
                let name: String = after_coding[1..]
                    .iter()
                    .copied()
                    .skip_while(|&b| b == b' ' || b == b'\t')
                    .take_while(|&b| {
                        b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.'
                    })
                    .map(|b| b as char)
                    .collect();
                if !name.is_empty() {
                    return Some(VirtualMachine::normalize_source_encoding(&name));
                }
            }
            None
        }

        let mut start = if source.starts_with(b"\xef\xbb\xbf") {
            3
        } else {
            0
        };
        for lineno in 1..=2 {
            let end = source[start..]
                .iter()
                .position(|&b| b == b'\r' || b == b'\n')
                .map_or(source.len(), |offset| start + offset);
            let line = &source[start..end];
            if let Some(name) = find_encoding_in_line(line) {
                return Some(SourceEncoding {
                    name,
                    lineno,
                    line: start..end,
                });
            }
            let first_nonspace = line
                .iter()
                .find(|&&b| b != b' ' && b != b'\t' && b != b'\x0c');
            if first_nonspace.is_some_and(|&b| b != b'#') || end == source.len() {
                return None;
            }
            start = end + 1;
            if source[end] == b'\r' && source.get(start) == Some(&b'\n') {
                start += 1;
            }
        }
        None
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

    /// Load the source line for a compiler SyntaxError. Tokenizer errors keep
    /// the in-memory line; compiler errors read the file named by the
    /// exception, and get None when that file cannot be opened.
    #[cfg(any(feature = "parser", feature = "compiler"))]
    pub(crate) fn program_text(&self, filename: &str, lineno: usize) -> Option<String> {
        if lineno == 0 {
            return None;
        }
        #[cfg(feature = "host_env")]
        {
            let buf = crate::host_env::fs::read(filename).ok()?;
            let decoded = {
                #[cfg(feature = "parser")]
                {
                    let encoding = Self::detect_source_encoding(&buf).map(|cookie| cookie.name);
                    if encoding.as_deref().is_none_or(Self::is_utf8_encoding) {
                        String::from_utf8_lossy(&buf).into_owned()
                    } else if encoding.as_deref() == Some("iso-8859-1") {
                        buf.iter().copied().map(char::from).collect()
                    } else {
                        let name = encoding.as_deref()?;
                        let bytes = self.ctx.new_bytes(buf);
                        self.state
                            .codec_registry
                            .decode_text(bytes.into(), name, None, self)
                            .ok()?
                            .to_string_lossy()
                            .into_owned()
                    }
                }
                #[cfg(not(feature = "parser"))]
                {
                    String::from_utf8_lossy(&buf).into_owned()
                }
            };
            let mut remaining = decoded.as_str();
            if remaining.starts_with('\u{feff}') {
                remaining = &remaining['\u{feff}'.len_utf8()..];
            }
            remaining
                .split_inclusive('\n')
                .nth(lineno.checked_sub(1)?)
                .map(str::to_owned)
        }
        #[cfg(not(feature = "host_env"))]
        {
            let _ = filename;
            None
        }
    }

    #[cfg(feature = "parser")]
    fn new_non_utf8_syntax_error(
        &self,
        filename: &str,
        src: &[u8],
        error_at: usize,
    ) -> PyBaseExceptionRef {
        let bad_byte = src[error_at];
        let line_start = src[..error_at]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |i| i + 1);
        let lineno = src[..error_at].iter().filter(|&&b| b == b'\n').count() + 1;
        let offset =
            core::str::from_utf8(&src[line_start..error_at]).map_or(1, |s| s.chars().count() + 1);
        let line_end = src[line_start..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(src.len(), |i| line_start + i);
        let text = String::from_utf8_lossy(&src[line_start..line_end]).into_owned();
        let in_file = if filename.starts_with('<') {
            String::new()
        } else {
            format!(" in file {filename}")
        };
        let msg = format!(
            "Non-UTF-8 code starting with '\\x{bad_byte:02x}'{in_file} \
             on line {lineno}, but no encoding declared; \
             see https://peps.python.org/pep-0263/ for details"
        );
        let location = self.ctx.new_tuple(vec![
            self.ctx.new_str(filename).into(),
            self.ctx.new_int(lineno).into(),
            self.ctx.new_int(offset).into(),
            self.ctx.new_str(text).into(),
            self.ctx.new_int(lineno).into(),
            self.ctx.new_int(offset).into(),
        ]);
        self.invoke_exception(
            self.ctx.exceptions.syntax_error,
            vec![self.ctx.new_str(msg.as_str()).into(), location.into()],
        )
        .unwrap_or_else(|_| {
            self.new_exception_msg(self.ctx.exceptions.syntax_error.to_owned(), msg.into())
        })
    }

    #[cfg(feature = "parser")]
    pub(crate) fn decode_source_bytes(
        &self,
        source: &[u8],
        filename: &str,
        ignore_cookie: bool,
    ) -> PyResult<String> {
        // `ignore_cookie` marks source that was handed over as text. A byte
        // order mark belongs to the byte encoding, so text keeps whatever it
        // was written with and the parser rejects a stray U+FEFF.
        let has_bom = !ignore_cookie && source.starts_with(b"\xef\xbb\xbf");
        let encoding = if ignore_cookie {
            None
        } else {
            Self::detect_source_encoding(source)
        };
        let encoding_name = encoding.as_ref().map(|cookie| cookie.name.as_str());
        let is_utf8 = encoding_name.is_none_or(Self::is_utf8_encoding);
        if has_bom && !is_utf8 {
            let cookie = encoding.as_ref().unwrap();
            let enc = cookie.name.as_str();
            let line = &source[cookie.line.clone()];
            let text = String::from_utf8_lossy(line).into_owned();
            // CPython's string tokenizer includes the line terminator for
            // the second coding line. Its range-error helper converts this
            // byte column by counting non-continuation bytes in the prefix.
            let end_byte_offset = line.len() + usize::from(cookie.lineno == 2);
            let end_offset = 1 + line[..end_byte_offset - 1]
                .iter()
                .filter(|&&byte| byte & 0xc0 != 0x80)
                .count();
            let msg = format!("encoding problem: {enc} with BOM");
            let location = self.ctx.new_tuple(vec![
                self.ctx.new_str(filename).into(),
                self.ctx.new_int(cookie.lineno).into(),
                self.ctx.new_int(0).into(),
                self.ctx.new_str(text).into(),
                self.ctx.new_int(cookie.lineno).into(),
                self.ctx.new_int(end_offset).into(),
            ]);
            return Err(self
                .invoke_exception(
                    self.ctx.exceptions.syntax_error,
                    vec![self.ctx.new_str(msg.as_str()).into(), location.into()],
                )
                .unwrap_or_else(|_| {
                    self.new_exception_msg(self.ctx.exceptions.syntax_error.to_owned(), msg.into())
                }));
        }

        let decoded = if is_utf8 {
            let src = if has_bom { &source[3..] } else { source };
            match core::str::from_utf8(src) {
                Ok(s) => s.to_owned(),
                Err(e) => {
                    return Err(self.new_non_utf8_syntax_error(filename, src, e.valid_up_to()));
                }
            }
        } else {
            let encoding = encoding_name.unwrap();
            let bytes = self.ctx.new_bytes(source.to_vec());
            let decoded = self
                .state
                .codec_registry
                .decode_text(bytes.into(), encoding, None, self)
                .map_err(|exc| {
                    if exc.fast_isinstance(self.ctx.exceptions.syntax_error) {
                        match exc
                            .as_object()
                            .set_attr("filename", self.ctx.new_str(filename), self)
                        {
                            Ok(()) => exc,
                            Err(error) => error,
                        }
                    } else if exc.fast_isinstance(self.ctx.exceptions.lookup_error)
                        || exc.fast_isinstance(self.ctx.exceptions.value_error)
                    {
                        let message = match exc.as_object().str(self) {
                            Ok(message) => message,
                            Err(error) => return error,
                        };
                        let location = self.ctx.new_tuple(vec![
                            self.ctx.new_str(filename).into(),
                            self.ctx.new_int(0).into(),
                            self.ctx.new_int(-1).into(),
                            self.ctx.none(),
                        ]);
                        self.invoke_exception(
                            self.ctx.exceptions.syntax_error,
                            vec![message.into(), location.into()],
                        )
                        .unwrap_or_else(|error| error)
                    } else {
                        exc
                    }
                })?;
            decoded.to_string_lossy().into_owned()
        };
        // Normalize the source before parsing so literal values, warning ranges,
        // and diagnostic text all refer to the same universal-newline input.
        Ok(if decoded.contains('\r') {
            decoded.replace("\r\n", "\n").replace('\r', "\n")
        } else {
            decoded
        })
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
        self.compile_string_object_with_flags_and_module(
            source,
            filename,
            start,
            flags,
            feature_version,
            optimize,
            None,
        )
    }

    #[cfg(feature = "parser")]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn compile_string_object_with_flags_and_module(
        &self,
        source: &[u8],
        filename: &str,
        start: i32,
        flags: i32,
        feature_version: i32,
        optimize: i32,
        module: Option<&Py<PyStr>>,
    ) -> PyResult<PyObjectRef> {
        use crate::stdlib::_ast;

        let cf = CompilerFlags::from_bits_retain(flags);
        let Some(start) = CompileStart::from_i32(start) else {
            return Err(self.new_system_error("Invalid start argument passed to Py_CompileString"));
        };
        let source =
            self.decode_source_bytes(source, filename, cf.contains(CompilerFlags::IGNORE_COOKIE))?;
        let source = source.as_str();
        let optimize = match optimize {
            -1 => self.state.config.settings.optimize.min(2),
            0..=2 => optimize as u8,
            _ => return Err(self.new_value_error("compile(): invalid optimize value")),
        };
        let allow_incomplete = cf.contains(CompilerFlags::ALLOW_INCOMPLETE_INPUT);
        let type_comments = cf.contains(CompilerFlags::TYPE_COMMENTS);
        let dont_imply_dedent = cf.contains(CompilerFlags::DONT_IMPLY_DEDENT);
        let is_ast_only = cf.contains(CompilerFlags::ONLY_AST);
        let optimized_ast = cf.contains(CompilerFlags::OPTIMIZED_AST);
        let future_features = compile_future_features_from_flags(flags);
        let target_version = if is_ast_only {
            Some(ruff_python_ast::PythonVersion {
                major: 3,
                minor: u8::try_from(feature_version).unwrap_or(crate::version::MINOR as u8),
            })
        } else {
            None
        };

        let mut grammar_warnings_emitted = false;
        let emitted_parser_escapes = if is_ast_only {
            self.emit_ast_parser_warnings(
                source,
                filename,
                None,
                module,
                start,
                escape_warnings::AstParserWarningOptions {
                    inherited_barry_as_flufl: future_features
                        .contains(crate::bytecode::CodeFlags::FUTURE_BARRY_AS_BDFL),
                    type_comments,
                },
            )
            .map_err(|warning| warning.into_pyexception(self, Some(source)))?
        } else if type_comments && start != CompileStart::FuncType {
            let mode = match start {
                CompileStart::Single => compiler::Mode::Single,
                CompileStart::File => compiler::Mode::Exec,
                CompileStart::Eval => compiler::Mode::Eval,
                CompileStart::FuncType => unreachable!(),
            };
            let emitted = self
                .try_emit_standard_grammar_warnings(
                    escape_warnings::WarningSource::new(source, filename),
                    module.map(AsObject::as_object),
                    mode,
                    future_features.contains(crate::bytecode::CodeFlags::FUTURE_BARRY_AS_BDFL),
                    true,
                    &[],
                )
                .map_err(|error| error.into_pyexception(self, Some(source)))?;
            grammar_warnings_emitted = emitted.is_some();
            emitted.unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut emit_ast_warning = |offset, ch| {
            if emitted_parser_escapes.binary_search(&offset).is_ok() {
                return Ok(());
            }
            escape_warnings::warn_invalid_escape_sequence(
                source,
                escape_warnings::InvalidEscape::Char { ch, offset },
                filename,
                None,
                module.map(AsObject::as_object),
                self,
            )
            .map_err(VmCompileError::Warning)
        };
        if is_ast_only {
            if start == CompileStart::FuncType {
                return _ast::parse_func_type(
                    self,
                    source,
                    filename,
                    optimize,
                    target_version,
                    &mut emit_ast_warning,
                )
                .map_err(|e| {
                    e.into_pyexception_maybe_incomplete(self, Some(source), allow_incomplete)
                });
            }
            let (parser_mode, interactive) = match start {
                CompileStart::Single => (ruff_python_parser::Mode::Module, true),
                CompileStart::File => (ruff_python_parser::Mode::Module, false),
                CompileStart::Eval => (ruff_python_parser::Mode::Expression, false),
                CompileStart::FuncType => unreachable!(),
            };
            let parsed = _ast::parse(
                self,
                source,
                filename,
                parser_mode,
                optimize,
                target_version,
                type_comments,
                optimized_ast,
                interactive,
                future_features,
                dont_imply_dedent,
                None,
                &mut emit_ast_warning,
            )
            .map_err(|e| {
                e.into_pyexception_maybe_incomplete(self, Some(source), allow_incomplete)
            })?;
            if start == CompileStart::Single {
                return _ast::wrap_interactive(self, &parsed);
            }
            return Ok(parsed);
        }

        if type_comments {
            let mut pending_ast_warnings = Vec::new();
            let mut type_comment_error_offset = None;
            let parser_mode = match start {
                CompileStart::Single | CompileStart::File => ruff_python_parser::Mode::Module,
                CompileStart::Eval => ruff_python_parser::Mode::Expression,
                CompileStart::FuncType => ruff_python_parser::Mode::Module,
            };
            let validation = _ast::parse(
                self,
                source,
                filename,
                parser_mode,
                optimize,
                None,
                type_comments,
                false,
                start == CompileStart::Single,
                future_features,
                dont_imply_dedent,
                Some(&mut type_comment_error_offset),
                |offset, ch| {
                    // Successful validation shares the ordinary collector's
                    // tokenizer/parser order instead of emitting format specs
                    // prematurely. Preserve reached callbacks if validation fails.
                    pending_ast_warnings.push((offset, ch));
                    Ok::<(), VmCompileError>(())
                },
            );
            if let Err(error) = validation {
                if type_comment_error_offset.is_some() && !grammar_warnings_emitted {
                    // The valid raw grammar stopped at a TYPE_COMMENT. Emit
                    // only reached parser actions, then tokenizer recovery,
                    // before returning the unchanged validation error.
                    self.emit_ast_parser_warnings(
                        source,
                        filename,
                        None,
                        module,
                        start,
                        escape_warnings::AstParserWarningOptions {
                            inherited_barry_as_flufl: future_features
                                .contains(crate::bytecode::CodeFlags::FUTURE_BARRY_AS_BDFL),
                            type_comments: true,
                        },
                    )
                    .map_err(|warning| warning.into_pyexception(self, Some(source)))?;
                }
                for (offset, ch) in pending_ast_warnings {
                    emit_ast_warning(offset, ch).map_err(|error| {
                        error.into_pyexception_maybe_incomplete(
                            self,
                            Some(source),
                            allow_incomplete,
                        )
                    })?;
                }
                return Err(error.into_pyexception_maybe_incomplete(
                    self,
                    Some(source),
                    allow_incomplete,
                ));
            }
        }

        let mode = match start {
            CompileStart::Single => compiler::Mode::Single,
            CompileStart::File => compiler::Mode::Exec,
            CompileStart::Eval => compiler::Mode::Eval,
            CompileStart::FuncType => compiler::Mode::BlockExpr,
        };
        let mut opts = self.compile_opts();
        opts.optimize = optimize;
        opts.allow_top_level_await = cf.contains(CompilerFlags::ALLOW_TOP_LEVEL_AWAIT);
        opts.future_features = future_features;
        opts.dont_imply_dedent = dont_imply_dedent;
        let code = if grammar_warnings_emitted {
            self.compile_after_parser_warnings_object(
                source,
                mode,
                filename.to_owned(),
                opts,
                module.map(AsObject::as_object),
            )
        } else if type_comments && start != CompileStart::FuncType {
            // None from early preparation can mean all actions were blocked by
            // TYPE_COMMENTS. Preserve that control on the final warning pass;
            // never reinterpret the same source with TYPE_COMMENTS disabled.
            self.emit_standard_parser_warnings_with_type_comments(
                escape_warnings::WarningSource::new(source, filename),
                module.map(AsObject::as_object),
                mode,
                future_features.contains(crate::bytecode::CodeFlags::FUTURE_BARRY_AS_BDFL),
                true,
                &[],
            )
            .and_then(|()| {
                self.compile_after_parser_warnings_object(
                    source,
                    mode,
                    filename.to_owned(),
                    opts,
                    module.map(AsObject::as_object),
                )
            })
        } else {
            self.compile_with_opts_and_module(source, mode, filename, opts, module, &[])
        }
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
        self.compile_with_opts_and_module(source, mode, source_path, opts, None, &[])
    }

    pub(crate) fn compile_symtable_with_module(
        &self,
        source: &str,
        mode: compiler::Mode,
        filename: &str,
        original_filename: &Py<PyStr>,
        module: Option<&Py<PyStr>>,
    ) -> Result<compiler::codegen::symboltable::SymbolTable, VmCompileError> {
        #[cfg(feature = "parser")]
        {
            let start = match mode {
                compiler::Mode::Exec | compiler::Mode::BlockExpr => CompileStart::File,
                compiler::Mode::Eval => CompileStart::Eval,
                compiler::Mode::Single => CompileStart::Single,
            };
            // A symbol table observes parser warnings, but does not compile
            // bytecode or emit code-generation and AST-preprocessing warnings.
            self.emit_ast_parser_warnings(
                source,
                filename,
                Some(original_filename),
                module,
                start,
                escape_warnings::AstParserWarningOptions::default(),
            )?;
        }
        #[cfg(not(feature = "parser"))]
        let _ = (module, original_filename);
        compiler::compile_symtable(source, mode, filename).map_err(VmCompileError::Compile)
    }

    pub(crate) fn compile_with_opts_and_module(
        &self,
        source: &str,
        mode: compiler::Mode,
        source_path: impl Into<String>,
        opts: CompileOpts,
        module: Option<&Py<PyStr>>,
        emitted_escapes: &[usize],
    ) -> Result<PyRef<PyCode>, VmCompileError> {
        self.compile_with_opts_and_module_object(
            source,
            mode,
            source_path,
            opts,
            module.map(AsObject::as_object),
            emitted_escapes,
        )
    }

    // eval()/exec() forward globals["__name__"] unchanged. In particular,
    // an absent name and an explicit None have different warning semantics.
    pub(crate) fn compile_with_opts_and_module_object(
        &self,
        source: &str,
        mode: compiler::Mode,
        source_path: impl Into<String>,
        opts: CompileOpts,
        module: Option<&PyObject>,
        emitted_escapes: &[usize],
    ) -> Result<PyRef<PyCode>, VmCompileError> {
        let source_path = source_path.into();
        #[cfg(not(feature = "parser"))]
        let _ = (module, emitted_escapes);
        #[cfg(feature = "parser")]
        {
            self.emit_standard_parser_warnings(
                source,
                &source_path,
                module,
                mode,
                opts.future_features
                    .contains(crate::bytecode::CodeFlags::FUTURE_BARRY_AS_BDFL),
                emitted_escapes,
            )?;
        }
        self.compile_after_parser_warnings_object(source, mode, source_path, opts, module)
    }

    pub(super) fn compile_after_parser_warnings(
        &self,
        source: &str,
        mode: compiler::Mode,
        source_path: impl Into<String>,
        opts: CompileOpts,
        module: Option<&Py<PyStr>>,
    ) -> Result<PyRef<PyCode>, VmCompileError> {
        self.compile_after_parser_warnings_object(
            source,
            mode,
            source_path,
            opts,
            module.map(AsObject::as_object),
        )
    }

    fn compile_after_parser_warnings_object(
        &self,
        source: &str,
        mode: compiler::Mode,
        source_path: impl Into<String>,
        opts: CompileOpts,
        module: Option<&PyObject>,
    ) -> Result<PyRef<PyCode>, VmCompileError> {
        let source_path = source_path.into();
        #[cfg(not(feature = "parser"))]
        let _ = module;
        #[cfg(feature = "parser")]
        let code = {
            // A warning the filter escalates to an exception is stashed here so
            // its precise category survives; codegen only sees an abort marker.
            let escalated: core::cell::Cell<Option<CompileWarningError>> =
                core::cell::Cell::new(None);
            let mut syntax_warning_handler = |location, message| {
                escape_warnings::warn_syntax_at_location(
                    &source_path,
                    module,
                    location,
                    message,
                    self,
                )
                .map_err(|warning| {
                    escalated.set(Some(warning));
                    // Recovered below via `escalated`, so this is never surfaced.
                    compiler::codegen::error::CodegenError {
                        location: Some(location),
                        end_location: None,
                        error: compiler::codegen::error::CodegenErrorType::SyntaxError(
                            String::new(),
                        ),
                        source_path: source_path.clone(),
                    }
                })
            };
            let result = compiler::compile_with_syntax_warning_handler(
                source,
                mode,
                &source_path,
                opts,
                &mut syntax_warning_handler,
            );
            match escalated.take() {
                Some(warning) => return Err(VmCompileError::Warning(warning)),
                None => result,
            }
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
    use ruff_text_size::TextRange;

    /// Borrowed source/name pair for the composition helpers, with no ownership
    /// or mode change and no additional parsing or allocation.
    #[derive(Clone, Copy)]
    pub(super) struct WarningSource<'a> {
        source: &'a str,
        filename: &'a str,
        original_filename: Option<&'a Py<PyStr>>,
    }

    impl<'a> WarningSource<'a> {
        pub(super) const fn new(source: &'a str, filename: &'a str) -> Self {
            Self {
                source,
                filename,
                original_filename: None,
            }
        }
    }

    fn line_offset_at(source: &str, offset: usize) -> (usize, usize) {
        let offset = offset.min(source.len());
        let prefix = &source[..offset];
        let lineno = prefix.bytes().filter(|&b| b == b'\n').count() + 1;
        let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
        let column = source[line_start..offset].chars().count() + 1;
        (lineno, column)
    }

    // Tokenizer errors use universal-newline physical positions even though
    // the caller retains the original source bytes for diagnostic text.
    fn tokenizer_line_offset_at(source: &str, offset: usize) -> (usize, usize) {
        let offset = offset.min(source.len());
        let bytes = source.as_bytes();
        let mut lineno = 1;
        let mut line_start = 0;
        for index in 0..offset {
            if bytes[index] == b'\r' {
                lineno += 1;
                line_start = index + 1;
            } else if bytes[index] == b'\n' {
                if index == 0 || bytes[index - 1] != b'\r' {
                    lineno += 1;
                }
                line_start = index + 1;
            }
        }
        (lineno, source[line_start..offset].chars().count() + 1)
    }

    fn compile_warning_error(
        exception: PyBaseExceptionRef,
        source: &str,
        filename: &str,
        original_filename: Option<&Py<PyStr>>,
        offset: usize,
    ) -> CompileWarningError {
        let (lineno, offset) = tokenizer_line_offset_at(source, offset);
        CompileWarningError {
            exception,
            filename: filename.to_owned(),
            original_filename: original_filename.map(ToOwned::to_owned),
            lineno,
            offset,
            replacement: None,
            diagnostic_text: None,
            omit_source_text: false,
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

    pub(super) enum InvalidEscape {
        Char { ch: char, offset: usize },
        Octal { digits: [u8; 3], offset: usize },
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum EscapeOrigin {
        Parser,
        Tokenizer,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum InterpolatedBodyDiagnostics {
        Legacy,
        TokenAnchored { implicit_final_newline: bool },
    }

    struct EscapeDiagnosticLocation {
        emission_offset: usize,
        lineno: usize,
        offset: usize,
        end_offset: usize,
        text_start: usize,
        text_end: usize,
        append_newline: bool,
    }

    #[derive(Clone, Copy)]
    struct InterpolatedTokenAnchor {
        token_start: usize,
        token_end: usize,
        quote_len: usize,
    }

    fn interpolated_body_anchor(source: &str, range: TextRange) -> Option<InterpolatedTokenAnchor> {
        let (_, closing_start) = content_bounds(source, range)?;
        Some(InterpolatedTokenAnchor {
            token_start: closing_start,
            token_end: range.end().to_usize(),
            quote_len: range.end().to_usize() - closing_start,
        })
    }

    struct SourceDiagnosticLine {
        start: usize,
        end: usize,
        terminated: bool,
    }

    struct NonAsciiScalar {
        start: usize,
        len: usize,
        excess_before: usize,
    }

    // Built lazily once per collector, only if an anchored warning is found.
    // ASCII contributes no scalar-index entries. Per-warning metadata stays
    // constant-sized even when many MIDDLE fragments share one long line.
    struct SourceDiagnosticIndex {
        lines: Vec<SourceDiagnosticLine>,
        non_ascii: Vec<NonAsciiScalar>,
    }

    impl SourceDiagnosticIndex {
        fn new(source: &str) -> Self {
            let bytes = source.as_bytes();
            let mut lines = Vec::new();
            let mut start = 0;
            let mut cursor = 0;
            while cursor < bytes.len() {
                if matches!(bytes[cursor], b'\r' | b'\n') {
                    lines.push(SourceDiagnosticLine {
                        start,
                        end: cursor,
                        terminated: true,
                    });
                    if bytes[cursor] == b'\r' && bytes.get(cursor + 1) == Some(&b'\n') {
                        cursor += 1;
                    }
                    start = cursor + 1;
                }
                cursor += 1;
            }
            lines.push(SourceDiagnosticLine {
                start,
                end: source.len(),
                terminated: false,
            });
            let mut excess = 0;
            let non_ascii = source
                .char_indices()
                .filter_map(|(start, ch)| {
                    if ch.is_ascii() {
                        return None;
                    }
                    let len = ch.len_utf8();
                    let entry = NonAsciiScalar {
                        start,
                        len,
                        excess_before: excess,
                    };
                    excess += len - 1;
                    Some(entry)
                })
                .collect();
            Self { lines, non_ascii }
        }

        // A partial final UTF8 scalar counts as one replacement character,
        // matching the parser's decode-with-replacement prefix conversion.
        fn character_count(&self, byte_offset: usize) -> usize {
            let count = self
                .non_ascii
                .partition_point(|entry| entry.start < byte_offset);
            let excess = count.checked_sub(1).map_or(0, |index| {
                let entry = &self.non_ascii[index];
                entry.excess_before + (byte_offset - entry.start).min(entry.len) - 1
            });
            byte_offset - excess
        }
    }

    #[derive(Clone, Copy)]
    struct InterpolatedLiteralContext<'a> {
        anchor: InterpolatedTokenAnchor,
        index: &'a core::cell::OnceCell<SourceDiagnosticIndex>,
        implicit_final_newline: bool,
    }

    // CPython decodes ordinary interpolated body constants with the END token,
    // not the MIDDLE token used for format-spec constants. Keep the actual escape
    // offset separate: it still owns deduplication and input boundaries.
    fn interpolated_literal_location(
        source: &str,
        start: usize,
        end: usize,
        escape_offset: usize,
        context: InterpolatedLiteralContext<'_>,
    ) -> EscapeDiagnosticLocation {
        debug_assert!(start <= escape_offset && escape_offset < end);
        let anchor = context.anchor;
        let index = context
            .index
            .get_or_init(|| SourceDiagnosticIndex::new(source));
        let line_index = index
            .lines
            .partition_point(|line| line.start <= anchor.token_start)
            - 1;
        let line = &index.lines[line_index];
        let token_lineno = line_index + 1;
        let mut lineno = token_lineno;
        let mut column = anchor.token_start - line.start;
        // string_parser.c rewrites non-ASCII codepoints to \UXXXXXXXX before
        // computing the first invalid escape pointer. Backslash + non-ASCII
        // uses \u005c followed by that rewrite. Include the invalid backslash,
        // not the following character to which first_invalid_escape points.
        let mut chars = source[start..=escape_offset].chars().peekable();
        while let Some(mut ch) = chars.next() {
            if ch == '\\' {
                column += if chars.peek().is_some_and(|ch| !ch.is_ascii()) {
                    6
                } else {
                    1
                };
                let Some(next) = chars.next() else {
                    break;
                };
                ch = next;
            }
            if ch == '\r' {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                ch = '\n';
            }
            if ch == '\n' {
                lineno += 1;
                column = 0;
            } else {
                column += if ch.is_ascii() { 1 } else { 10 };
            }
        }
        if lineno == token_lineno {
            column += anchor.quote_len;
        }
        let current_line_index = index
            .lines
            .partition_point(|line| line.start <= anchor.token_end)
            - 1;
        // pegen_errors.c uses the tokenizer's current line when the diagnostic
        // is at or beyond it. Otherwise it looks up the earlier source line,
        // without its LF. END anchors always take the current-line branch;
        // multiline format-spec MIDDLE tokens can take either branch.
        let use_current_line = current_line_index < lineno;
        let line = &index.lines[if use_current_line {
            current_line_index
        } else {
            lineno - 1
        }];
        let append_newline =
            use_current_line && (line.terminated || context.implicit_final_newline);
        // A physical CR/CRLF becomes LF. Only file input synthesizes a final
        // LF when absent. pegen.c then clamps to that line plus its NUL.
        let content_len = line.end - line.start;
        let base_characters = index.character_count(line.start);
        let character_offset = |offset: usize| {
            index.character_count(line.start + offset.min(content_len)) - base_characters
                + usize::from(append_newline && offset > content_len)
                + usize::from(offset > content_len + usize::from(append_newline))
        };
        EscapeDiagnosticLocation {
            emission_offset: anchor.token_end,
            lineno,
            offset: character_offset(column),
            end_offset: character_offset(column + 2),
            text_start: line.start,
            text_end: line.end,
            append_newline,
        }
    }

    enum ParserWarning {
        /// The escape, where it is reported from, and for a bytes literal
        /// escape on the literal's first line, the error column that
        /// string_parser.c reports.
        Escape(
            InvalidEscape,
            EscapeOrigin,
            Option<usize>,
            Option<EscapeDiagnosticLocation>,
        ),
        RelativeLazy {
            action: ruff_python_parser::RelativeLazyImportWarning,
            // True prompt input returns immediately, without tokenizer recovery.
            recovery: Option<GrammarRecoveryPlan>,
            index: alloc::sync::Arc<SourceDiagnosticIndex>,
            mode: compiler::Mode,
        },
        Numeric {
            offset: usize,
            last_offset: usize,
            message: String,
            in_interpolation: bool,
        },
    }

    impl ParserWarning {
        fn offset(&self) -> usize {
            match self {
                Self::Escape(escape, ..) => escape.offset(),
                Self::Numeric { offset, .. } => *offset,
                Self::RelativeLazy { action, .. } => action.module.start().to_usize(),
            }
        }

        fn emission_offset(&self) -> usize {
            match self {
                // The grammar decodes body constants only after consuming the
                // END token. Expressions and format specs have already been
                // parsed by then, even if their escapes occur later in source.
                Self::Escape(_, _, _, Some(diagnostic)) => diagnostic.emission_offset,
                Self::RelativeLazy { action, .. } => action.tokenizer_end.to_usize(),
                _ => self.offset(),
            }
        }
    }

    impl InvalidEscape {
        fn offset(&self) -> usize {
            match self {
                Self::Char { offset, .. } | Self::Octal { offset, .. } => *offset,
            }
        }

        fn octal_text(digits: [u8; 3]) -> [char; 3] {
            [digits[0] as char, digits[1] as char, digits[2] as char]
        }

        fn warning_message(&self) -> String {
            match self {
                Self::Char { ch, .. } => format!(
                    "\"\\{ch}\" is an invalid escape sequence. \
                     Such sequences will not work in the future. \
                     Did you mean \"\\\\{ch}\"? A raw string is also an option."
                ),
                Self::Octal { digits, .. } => {
                    let [a, b, c] = Self::octal_text(*digits);
                    format!(
                        "\"\\{a}{b}{c}\" is an invalid octal escape sequence. \
                         Such sequences will not work in the future. \
                         Did you mean \"\\\\{a}{b}{c}\"? A raw string is also an option."
                    )
                }
            }
        }

        fn syntax_error_message(&self) -> String {
            match self {
                Self::Char { ch, .. } => format!(
                    "\"\\{ch}\" is an invalid escape sequence. \
                     Did you mean \"\\\\{ch}\"? A raw string is also an option."
                ),
                Self::Octal { digits, .. } => {
                    let [a, b, c] = Self::octal_text(*digits);
                    format!(
                        "\"\\{a}{b}{c}\" is an invalid octal escape sequence. \
                         Did you mean \"\\\\{a}{b}{c}\"? A raw string is also an option."
                    )
                }
            }
        }
    }

    /// Scan `source[start..end]` for the first invalid escape sequence.
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
    ) -> Option<InvalidEscape> {
        let raw = &source[start..end];
        let mut chars = raw.char_indices().peekable();
        while let Some((i, ch)) = chars.next() {
            if ch != '\\' {
                continue;
            }
            let Some((_, next)) = chars.next() else {
                break;
            };
            let offset = start + i;
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
                    let mut digits = [next as u8, 0, 0];
                    let mut len = 1;
                    for _ in 0..2 {
                        if matches!(chars.peek(), Some(&(_, '0'..='7'))) {
                            let (_, digit) = chars.next().unwrap();
                            digits[len] = digit as u8;
                            len += 1;
                        } else {
                            break;
                        }
                    }
                    if len == 3 {
                        let value = ((digits[0] - b'0') as u16) * 64
                            + ((digits[1] - b'0') as u16) * 8
                            + (digits[2] - b'0') as u16;
                        if value > 0o377 {
                            return Some(InvalidEscape::Octal { digits, offset });
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
                // string_parser.c protects a backslash before a non-ASCII
                // codepoint as \\u005c before passing the rewritten buffer to
                // the Unicode escape decoder. It cannot be an invalid escape.
                ch if !is_bytes && !ch.is_ascii() => true,
                _ => false,
            };
            if !valid {
                return Some(InvalidEscape::Char { ch: next, offset });
            }
        }
        None
    }

    /// Emit `SyntaxWarning` for an invalid escape sequence.
    ///
    /// `warn_invalid_escape_sequence()` in `Parser/string_parser.c`
    pub(super) fn warn_invalid_escape_sequence(
        source: &str,
        escape: InvalidEscape,
        filename: &str,
        original_filename: Option<&Py<PyStr>>,
        module: Option<&PyObject>,
        vm: &VirtualMachine,
    ) -> Result<(), CompileWarningError> {
        warn_invalid_escape_sequence_at(
            source,
            escape,
            filename,
            original_filename,
            module,
            None,
            vm,
        )
    }

    fn warn_invalid_escape_sequence_at(
        source: &str,
        escape: InvalidEscape,
        filename: &str,
        original_filename: Option<&Py<PyStr>>,
        module: Option<&PyObject>,
        diagnostic: Option<EscapeDiagnosticLocation>,
        vm: &VirtualMachine,
    ) -> Result<(), CompileWarningError> {
        let (lineno, column) = diagnostic.as_ref().map_or_else(
            || line_offset_at(source, escape.offset()),
            |location| (location.lineno, location.offset),
        );
        let end_offset = diagnostic
            .as_ref()
            .map_or(column + 2, |location| location.end_offset);
        let warning = escape.warning_message();
        let syntax_error = escape.syntax_error_message();
        let fname =
            original_filename.map_or_else(|| vm.ctx.new_str(filename), |name| name.to_owned());
        warn::warn_explicit(
            Some(vm.ctx.exceptions.syntax_warning.to_owned()),
            vm.ctx.new_str(warning).into(),
            fname,
            lineno,
            module.map(ToOwned::to_owned),
            vm.ctx.none(),
            None,
            None,
            vm,
        )
        .map_err(|exception| CompileWarningError {
            exception,
            filename: filename.to_owned(),
            original_filename: original_filename.map(ToOwned::to_owned),
            lineno,
            offset: column,
            replacement: Some(SyntaxErrorReplacement {
                message: syntax_error,
                end_offset,
            }),
            diagnostic_text: diagnostic.map(|location| {
                let mut text = source[location.text_start..location.text_end].to_owned();
                if location.append_newline {
                    text.push('\n');
                }
                text
            }),
            omit_source_text: false,
        })
    }

    fn warn_syntax_at_offset(
        source: &str,
        filename: &str,
        original_filename: Option<&Py<PyStr>>,
        module: Option<&PyObject>,
        offset: usize,
        message: String,
        vm: &VirtualMachine,
    ) -> Result<(), CompileWarningError> {
        let (lineno, _) = tokenizer_line_offset_at(source, offset);
        let fname =
            original_filename.map_or_else(|| vm.ctx.new_str(filename), |name| name.to_owned());
        let message = vm.ctx.new_str(message);
        warn::warn_explicit(
            Some(vm.ctx.exceptions.syntax_warning.to_owned()),
            message.into(),
            fname,
            lineno,
            module.map(ToOwned::to_owned),
            vm.ctx.none(),
            None,
            None,
            vm,
        )
        .map_err(|err| compile_warning_error(err, source, filename, original_filename, offset))
    }

    fn warn_numeric_literal(
        source: &str,
        filename: &str,
        original_filename: Option<&Py<PyStr>>,
        module: Option<&PyObject>,
        last_offset: usize,
        message: String,
        vm: &VirtualMachine,
    ) -> Result<(), CompileWarningError> {
        warn_syntax_at_offset(
            source,
            filename,
            original_filename,
            module,
            last_offset,
            message.clone(),
            vm,
        )
        .map_err(|mut error| {
            // Tokenizer numeric warnings become a zero-width
            // error at the literal's last character, in character
            // columns rather than grammar-action byte columns.
            error.replacement = Some(SyntaxErrorReplacement {
                message,
                end_offset: error.offset,
            });
            // helpers.c:_syntaxerror_range uses the physical
            // tokenizer line without its terminating LF in
            // every compile mode, not only interactive input.
            let line_start = source[..last_offset]
                .rfind(['\r', '\n'])
                .map_or(0, |index| index + 1);
            let line_end = source[last_offset..]
                .find(['\r', '\n'])
                .map_or(source.len(), |index| last_offset + index);
            error.diagnostic_text = Some(source[line_start..line_end].to_owned());
            error
        })
    }

    pub(super) fn warn_syntax_at_location(
        filename: &str,
        module: Option<&PyObject>,
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
            module.map(ToOwned::to_owned),
            vm.ctx.none(),
            None,
            None,
            vm,
        )
        .map_err(|exception| CompileWarningError {
            exception,
            filename: filename.to_owned(),
            original_filename: None,
            lineno: location.line.get(),
            offset: location.character_offset.get(),
            replacement: None,
            diagnostic_text: None,
            omit_source_text: false,
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

    #[derive(Clone, Copy)]
    enum StringPrefix {
        None,
        Invalid,
        Valid {
            is_raw: bool,
            is_bytes: bool,
            is_interpolated: bool,
        },
    }

    fn is_string_prefix_letter(byte: u8) -> bool {
        matches!(
            byte,
            b'r' | b'R' | b'b' | b'B' | b'u' | b'U' | b'f' | b'F' | b't' | b'T'
        )
    }

    fn prefix_continues_identifier(byte: u8) -> bool {
        byte == b'_' || byte.is_ascii_alphabetic() || byte >= 0x80
    }

    fn two_char_prefix(first: u8, second: u8) -> Option<(bool, bool, bool)> {
        match [first.to_ascii_lowercase(), second.to_ascii_lowercase()] {
            [b'r', b'f' | b't'] | [b'f' | b't', b'r'] => Some((true, false, true)),
            [b'r', b'b'] | [b'b', b'r'] => Some((true, true, false)),
            _ => None,
        }
    }

    /// Prefix letters immediately before a quote, only when they form their own token.
    fn string_prefix(bytes: &[u8], quote_index: usize) -> StringPrefix {
        let mut taken = [0u8; 2];
        let mut count = 0;
        let mut index = quote_index;
        while index > 0 && count < 2 {
            let byte = bytes[index - 1];
            if !is_string_prefix_letter(byte) {
                break;
            }
            taken[count] = byte;
            count += 1;
            index -= 1;
        }
        if count == 0 {
            return StringPrefix::None;
        }
        if index > 0 && prefix_continues_identifier(bytes[index - 1]) {
            return StringPrefix::None;
        }
        if count == 2 {
            return match two_char_prefix(taken[1], taken[0]) {
                Some((is_raw, is_bytes, is_interpolated)) => StringPrefix::Valid {
                    is_raw,
                    is_bytes,
                    is_interpolated,
                },
                None => StringPrefix::Invalid,
            };
        }
        let (is_raw, is_bytes, is_interpolated) = match taken[0] {
            b'r' | b'R' => (true, false, false),
            b'b' | b'B' => (false, true, false),
            b'f' | b'F' | b't' | b'T' => (false, false, true),
            _ => (false, false, false),
        };
        StringPrefix::Valid {
            is_raw,
            is_bytes,
            is_interpolated,
        }
    }

    fn text_range_from_bounds(start: usize, end: usize) -> TextRange {
        TextRange::new(
            ruff_text_size::TextSize::try_from(start).unwrap_or_default(),
            ruff_text_size::TextSize::try_from(end).unwrap_or_default(),
        )
    }

    fn collect_quoted_literal_warning(
        source: &str,
        literal_start: usize,
        end: usize,
        is_bytes: bool,
        warnings: &mut Vec<ParserWarning>,
    ) {
        if let Some((start, content_end)) =
            content_bounds(source, text_range_from_bounds(literal_start, end))
            && let Some(escape) = first_invalid_escape(source, start, content_end, is_bytes)
        {
            let before_escape = &source[start..escape.offset()];
            // string_parser.c counts leading quotes from the token start.
            // A bytes prefix contributes none to its first-line error range.
            // Keep the actual escape offset for warning order and deduplication.
            let error_column = (is_bytes && !before_escape.contains(['\r', '\n'])).then(|| {
                let (_, column) = line_offset_at(source, escape.offset());
                column.saturating_sub(start - literal_start)
            });
            warnings.push(ParserWarning::Escape(
                escape,
                EscapeOrigin::Parser,
                error_column,
                None,
            ));
        }
    }

    fn collect_fstring_literal_warning(
        source: &str,
        start: usize,
        end: usize,
        warnings: &mut Vec<ParserWarning>,
        anchor: Option<InterpolatedLiteralContext<'_>>,
    ) {
        if start >= end || end > source.len() {
            return;
        }
        if let Some(escape) = first_invalid_escape(source, start, end, false) {
            let origin = if matches!(&escape, InvalidEscape::Char { ch: '{' | '}', .. }) {
                EscapeOrigin::Tokenizer
            } else {
                EscapeOrigin::Parser
            };
            let diagnostic = if origin == EscapeOrigin::Parser {
                anchor.map(|anchor| {
                    interpolated_literal_location(source, start, end, escape.offset(), anchor)
                })
            } else {
                None
            };
            warnings.push(ParserWarning::Escape(escape, origin, None, diagnostic));
            return;
        }
        // In CPython, _PyTokenizer_warn_invalid_escape_sequence handles
        // `\{` and `\}` for FSTRING_MIDDLE/FSTRING_END tokens.  Ruff
        // splits the literal element before the interpolation delimiter,
        // so the `\` sits at the end of the literal range and the `{`/`}`
        // sits just after it.  Only warn when the number of trailing
        // backslashes is odd (an even count means they are all escaped).
        let trailing_bs = source.as_bytes()[start..end]
            .iter()
            .rev()
            .take_while(|&&byte| byte == b'\\')
            .count();
        if trailing_bs % 2 == 1
            && let Some(&after) = source.as_bytes().get(end)
            && (after == b'{' || after == b'}')
        {
            warnings.push(ParserWarning::Escape(
                InvalidEscape::Char {
                    ch: after as char,
                    offset: end - 1,
                },
                EscapeOrigin::Tokenizer,
                None,
                None,
            ));
        }
    }

    fn find_interpolation_end(bytes: &[u8], open: usize, limit: usize) -> usize {
        let mut index = open + 1;
        let mut depth: u32 = 1;
        let mut paren: u32 = 0;
        let mut bracket: u32 = 0;
        while index < limit {
            match bytes[index] {
                b'#' if paren == 0 && bracket == 0 && depth == 1 => {
                    while index < limit && bytes[index] != b'\n' {
                        index += 1;
                    }
                }
                b'\'' | b'"' => {
                    index = skip_quoted_string(bytes, index).max(index + 1);
                }
                b'(' => {
                    paren += 1;
                    index += 1;
                }
                b')' => {
                    paren = paren.saturating_sub(1);
                    index += 1;
                }
                b'[' => {
                    bracket += 1;
                    index += 1;
                }
                b']' => {
                    bracket = bracket.saturating_sub(1);
                    index += 1;
                }
                b'{' => {
                    depth += 1;
                    index += 1;
                }
                b'}' => {
                    depth -= 1;
                    index += 1;
                    if depth == 0 {
                        return index;
                    }
                }
                b'\\' => index = (index + 2).min(limit),
                _ => index += 1,
            }
        }
        limit
    }

    fn scan_interpolated_content(
        source: &str,
        start: usize,
        end: usize,
        is_raw: bool,
        warnings: &mut Vec<ParserWarning>,
    ) {
        let bytes = source.as_bytes();
        let mut index = start;
        let mut literal_start = start;
        while index < end {
            match bytes[index] {
                b'{' if bytes.get(index + 1) == Some(&b'{') => {
                    index += 2;
                }
                b'{' => {
                    if !is_raw {
                        collect_fstring_literal_warning(
                            source,
                            literal_start,
                            index,
                            warnings,
                            None,
                        );
                    }
                    let close = find_interpolation_end(bytes, index, end);
                    let body_end = if close > index + 1 { close - 1 } else { close };
                    collect_string_escape_warnings_in_range(
                        source,
                        index + 1,
                        body_end,
                        Some(is_raw),
                        warnings,
                    );
                    index = close.max(index + 1);
                    literal_start = index;
                }
                b'}' if bytes.get(index + 1) == Some(&b'}') => {
                    index += 2;
                }
                b'\\' => index = (index + 2).min(end),
                _ => index += 1,
            }
        }
        if !is_raw {
            collect_fstring_literal_warning(source, literal_start, end, warnings, None);
        }
    }

    /// Scan quoted literals without a successful parse, matching the tokenizer
    /// path that warns before the parser rejects the rest of the source.
    fn collect_string_escape_warnings_in_range(
        source: &str,
        start: usize,
        end: usize,
        format_spec_raw: Option<bool>,
        warnings: &mut Vec<ParserWarning>,
    ) {
        let bytes = source.as_bytes();
        let end = end.min(bytes.len());
        let mut index = start;
        let mut paren: u32 = 0;
        let mut bracket: u32 = 0;
        let mut brace: u32 = 0;
        while index < end {
            match bytes[index] {
                b'#' => {
                    while index < end && bytes[index] != b'\n' {
                        index += 1;
                    }
                }
                b'\'' | b'"' => {
                    let prefix = string_prefix(bytes, index);
                    let quote_end = skip_quoted_string(bytes, index).min(bytes.len());
                    match prefix {
                        StringPrefix::Valid {
                            is_raw,
                            is_bytes,
                            is_interpolated,
                        } => {
                            if is_interpolated {
                                if let Some((content_start, content_end)) =
                                    content_bounds(source, text_range_from_bounds(index, quote_end))
                                {
                                    scan_interpolated_content(
                                        source,
                                        content_start,
                                        content_end.min(end),
                                        is_raw,
                                        warnings,
                                    );
                                }
                            } else if !is_raw {
                                collect_quoted_literal_warning(
                                    // Non-raw ordinary prefixes are one-byte b/u.
                                    source,
                                    index - 1,
                                    quote_end,
                                    is_bytes,
                                    warnings,
                                );
                            }
                        }
                        StringPrefix::None => {
                            collect_quoted_literal_warning(
                                source, index, quote_end, false, warnings,
                            );
                        }
                        StringPrefix::Invalid => {}
                    }
                    index = quote_end.max(index + 1);
                }
                b':' if let Some(is_raw) = format_spec_raw
                    && paren == 0
                    && bracket == 0
                    && brace == 0 =>
                {
                    scan_interpolated_content(source, index + 1, end, is_raw, warnings);
                    return;
                }
                b'(' if format_spec_raw.is_some() => {
                    paren += 1;
                    index += 1;
                }
                b')' if format_spec_raw.is_some() => {
                    paren = paren.saturating_sub(1);
                    index += 1;
                }
                b'[' if format_spec_raw.is_some() => {
                    bracket += 1;
                    index += 1;
                }
                b']' if format_spec_raw.is_some() => {
                    bracket = bracket.saturating_sub(1);
                    index += 1;
                }
                b'{' if format_spec_raw.is_some() => {
                    brace += 1;
                    index += 1;
                }
                b'}' if format_spec_raw.is_some() => {
                    brace = brace.saturating_sub(1);
                    index += 1;
                }
                _ => index += 1,
            }
        }
    }

    fn collect_numeric_literal_warnings(source: &str, warnings: &mut Vec<ParserWarning>) {
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
                        warnings.push(ParserWarning::Numeric {
                            offset: index,
                            last_offset: end - 1,
                            message: format!("invalid {kind} literal"),
                            in_interpolation: false,
                        });
                    }
                    index = end.max(index + 1);
                }
                _ => index += 1,
            }
        }
    }

    fn collect_interpolated_fragment_warning(
        source: &str,
        start: usize,
        end: usize,
        warnings: &mut Vec<ParserWarning>,
        anchor: Option<InterpolatedLiteralContext<'_>>,
    ) {
        if let Some(escape) = first_invalid_escape(source, start, end, false)
            && !matches!(escape, InvalidEscape::Char { ch: '{' | '}', .. })
        {
            // The tokenizer reports braces independently. The decoder still
            // stops at its first invalid escape, including an already-reported
            // brace; it must not search past that brace for a parser warning.
            let diagnostic = anchor.map(|context| {
                interpolated_literal_location(source, start, end, escape.offset(), context)
            });
            warnings.push(ParserWarning::Escape(
                escape,
                EscapeOrigin::Parser,
                None,
                diagnostic,
            ));
        }
    }

    fn tokenizer_brace_location(
        index: &SourceDiagnosticIndex,
        escape_offset: usize,
        cursor_after_brace: usize,
    ) -> EscapeDiagnosticLocation {
        let brace_offset = cursor_after_brace - 1;
        let line_index = index
            .lines
            .partition_point(|line| line.start <= brace_offset)
            - 1;
        let line = &index.lines[line_index];
        // helpers.c reports the cursor after the brace, a zero-width range,
        // and the physical line without LF. This is the existing v8 formula.
        let column = index.character_count(cursor_after_brace) - index.character_count(line.start);
        EscapeDiagnosticLocation {
            emission_offset: escape_offset,
            lineno: line_index + 1,
            offset: column,
            end_offset: column,
            text_start: line.start,
            text_end: line.end,
            append_newline: false,
        }
    }

    fn collect_interpolated_tokenizer_warnings(
        source: &str,
        start: usize,
        end: usize,
        index: &core::cell::OnceCell<SourceDiagnosticIndex>,
        warnings: &mut Vec<ParserWarning>,
    ) {
        let bytes = source.as_bytes();
        let mut cursor = start;
        while cursor < end {
            if bytes[cursor] != b'\\' {
                cursor += source[cursor..end].chars().next().unwrap().len_utf8();
                continue;
            }
            let escape_offset = cursor;
            cursor += 1;
            if let Some(&brace) = bytes.get(cursor)
                && matches!(brace, b'{' | b'}')
            {
                let index = index.get_or_init(|| SourceDiagnosticIndex::new(source));
                warnings.push(ParserWarning::Escape(
                    InvalidEscape::Char {
                        ch: brace as char,
                        offset: escape_offset,
                    },
                    EscapeOrigin::Tokenizer,
                    None,
                    Some(tokenizer_brace_location(index, escape_offset, cursor + 1)),
                ));
            } else if cursor < end {
                cursor += source[cursor..end].chars().next().unwrap().len_utf8();
                if bytes.get(cursor - 1) == Some(&b'\r') && bytes.get(cursor) == Some(&b'\n') {
                    cursor += 1;
                }
            }
        }
    }

    fn collect_interpolated_literal_warnings(
        source: &str,
        start: usize,
        end: usize,
        body_anchor: Option<InterpolatedTokenAnchor>,
        index: &core::cell::OnceCell<SourceDiagnosticIndex>,
        implicit_final_newline: bool,
        warnings: &mut Vec<ParserWarning>,
    ) {
        // CPython emits a separate MIDDLE token after doubled braces and named
        // Unicode escapes. Doubled braces are only literal outside format specs.
        // Ruff can keep these inside one literal element.
        let bytes = source.as_bytes();
        let mut collect_fragment = |start: usize, end: usize| {
            let anchor = body_anchor.unwrap_or_else(|| {
                // string_parser.c counts the initial run of quote characters
                // in the actual token, even for format-spec MIDDLE tokens.
                let quote_len = bytes.get(start).map_or(0, |quote| {
                    bytes[start..end]
                        .iter()
                        .take_while(|&&byte| matches!(byte, b'\'' | b'"'))
                        .filter(|&byte| byte == quote)
                        .count()
                });
                InterpolatedTokenAnchor {
                    token_start: start,
                    token_end: end,
                    quote_len,
                }
            });
            collect_interpolated_fragment_warning(
                source,
                start,
                end,
                warnings,
                Some(InterpolatedLiteralContext {
                    anchor,
                    index,
                    implicit_final_newline,
                }),
            );
        };
        let mut fragment_start = start;
        let mut cursor = start;
        while cursor < end {
            match bytes[cursor] {
                b'{' | b'}'
                    if body_anchor.is_some() && bytes.get(cursor + 1) == Some(&bytes[cursor]) =>
                {
                    collect_fragment(fragment_start, cursor + 1);
                    cursor += 2;
                    fragment_start = cursor;
                }
                b'\\' => {
                    if bytes.get(cursor + 1) == Some(&b'N')
                        && bytes.get(cursor + 2) == Some(&b'{')
                        && let Some(close) = source[cursor + 3..end].find('}')
                    {
                        cursor += 3 + close + 1;
                        collect_fragment(fragment_start, cursor);
                        fragment_start = cursor;
                        continue;
                    }
                    cursor += 1;
                    if cursor < end && !matches!(bytes[cursor], b'{' | b'}') {
                        // Paired backslashes must not turn the second slash
                        // into a new named-Unicode escape introducer.
                        cursor += source[cursor..end].chars().next().unwrap().len_utf8();
                        if bytes.get(cursor - 1) == Some(&b'\r')
                            && bytes.get(cursor) == Some(&b'\n')
                        {
                            cursor += 1;
                        }
                    }
                }
                _ => cursor += source[cursor..end].chars().next().unwrap().len_utf8(),
            }
        }
        collect_fragment(fragment_start, end);
    }

    struct EscapeWarningVisitor<'a, 'w> {
        source: &'a str,
        warnings: &'w mut Vec<ParserWarning>,
        ast_only: bool,
        interpolated_body_diagnostics: InterpolatedBodyDiagnostics,
        diagnostic_index: core::cell::OnceCell<SourceDiagnosticIndex>,
        interpolation_depth: usize,
        /// This pass runs before the compile that rejects an over-nested
        /// tree, so it has to stop itself.
        depth: usize,
        depth_limit: usize,
    }

    impl<'a> EscapeWarningVisitor<'a, '_> {
        /// Check a quoted string/bytes literal for invalid escapes.
        /// The range must include the prefix and quote delimiters.
        fn check_quoted_literal(&mut self, range: TextRange, is_bytes: bool) {
            let previous_len = self.warnings.len();
            collect_quoted_literal_warning(
                self.source,
                range.start().to_usize(),
                range.end().to_usize(),
                is_bytes,
                self.warnings,
            );
            if !is_bytes
                && self.warnings.len() != previous_len
                && let InterpolatedBodyDiagnostics::TokenAnchored {
                    implicit_final_newline,
                } = self.interpolated_body_diagnostics
                && let Some(ParserWarning::Escape(escape, _, _, diagnostic)) =
                    self.warnings.last_mut()
            {
                let index = self
                    .diagnostic_index
                    .get_or_init(|| SourceDiagnosticIndex::new(self.source));
                let line = index.lines.last().unwrap();
                if escape.offset() >= line.start && !line.terminated {
                    // Use one physical index for warning/error coordinates
                    // and final-line text, including bare CR. File input adds
                    // a final LF; eval/single leave this line unterminated.
                    // Keep existing LF/CRLF columns and source emission order.
                    let lineno = index.lines.len();
                    let column = index.character_count(escape.offset())
                        - index.character_count(line.start)
                        + 1;
                    *diagnostic = Some(EscapeDiagnosticLocation {
                        emission_offset: escape.offset(),
                        lineno,
                        offset: column,
                        end_offset: column + 2,
                        text_start: line.start,
                        text_end: line.end,
                        append_newline: implicit_final_newline,
                    });
                }
            }
        }

        /// Check an f-string literal element for invalid escapes.
        /// The range covers content only (no prefix/quotes).
        ///
        /// Also handles `\{` / `\}` at the literal–interpolation boundary,
        /// equivalent to `_PyTokenizer_warn_invalid_escape_sequence` handling
        /// `FSTRING_MIDDLE` / `FSTRING_END` tokens.
        fn check_fstring_literal(
            &mut self,
            range: TextRange,
            anchor: Option<InterpolatedTokenAnchor>,
        ) {
            if let InterpolatedBodyDiagnostics::TokenAnchored {
                implicit_final_newline,
            } = self.interpolated_body_diagnostics
            {
                collect_interpolated_tokenizer_warnings(
                    self.source,
                    range.start().to_usize(),
                    range.end().to_usize(),
                    &self.diagnostic_index,
                    self.warnings,
                );
                collect_interpolated_literal_warnings(
                    self.source,
                    range.start().to_usize(),
                    range.end().to_usize(),
                    anchor,
                    &self.diagnostic_index,
                    implicit_final_newline,
                    self.warnings,
                );
            } else {
                collect_fstring_literal_warning(
                    self.source,
                    range.start().to_usize(),
                    range.end().to_usize(),
                    self.warnings,
                    None,
                );
            }
        }

        /// Visit f-string elements, checking literals and recursing into
        /// interpolation expressions and format specs.
        fn visit_fstring_elements(
            &mut self,
            elements: &'a ast::InterpolatedStringElements,
            raw: bool,
            anchor: Option<InterpolatedTokenAnchor>,
        ) {
            for element in elements {
                match element {
                    ast::InterpolatedStringElement::Literal(lit) => {
                        if !raw {
                            self.check_fstring_literal(lit.range, anchor);
                        }
                    }
                    ast::InterpolatedStringElement::Interpolation(interp) => {
                        self.interpolation_depth += 1;
                        self.visit_expr(&interp.expression);
                        if let Some(spec) = &interp.format_spec {
                            self.visit_fstring_elements(&spec.elements, raw, None);
                        }
                        self.interpolation_depth -= 1;
                    }
                }
            }
        }
    }

    impl<'a> Visitor<'a> for EscapeWarningVisitor<'a, '_> {
        fn visit_expr(&mut self, expr: &'a ast::Expr) {
            match expr {
                ast::Expr::NumberLiteral(number)
                    if matches!(
                        self.interpolated_body_diagnostics,
                        InterpolatedBodyDiagnostics::TokenAnchored { .. }
                    ) =>
                {
                    let start = number.range.start().to_usize();
                    let bytes = self.source.as_bytes();
                    if let Some((kind, end)) = number_literal_end(bytes, start)
                        && numeric_keyword_suffix(&bytes[end..])
                    {
                        self.warnings.push(ParserWarning::Numeric {
                            offset: start,
                            last_offset: end - 1,
                            message: format!("invalid {kind} literal"),
                            in_interpolation: self.interpolation_depth != 0,
                        });
                    }
                }
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
                    for part in &fstring_expr.value {
                        match part {
                            ast::FStringPartRef::Literal(string_lit) => {
                                // Plain string part in f-string concatenation
                                if !matches!(
                                    string_lit.flags.prefix(),
                                    ast::str_prefix::StringLiteralPrefix::Raw { .. }
                                ) {
                                    self.check_quoted_literal(string_lit.range, false);
                                }
                            }
                            ast::FStringPartRef::FString(fstring) => {
                                if !self.ast_only
                                    && matches!(
                                        fstring.flags.prefix(),
                                        ast::str_prefix::FStringPrefix::Raw { .. }
                                    )
                                {
                                    continue;
                                }
                                self.visit_fstring_elements(
                                    &fstring.elements,
                                    matches!(
                                        fstring.flags.prefix(),
                                        ast::str_prefix::FStringPrefix::Raw { .. }
                                    ),
                                    interpolated_body_anchor(self.source, fstring.range),
                                );
                            }
                        }
                    }
                }
                ast::Expr::TString(tstring) if self.ast_only => {
                    for part in tstring.value.as_slice() {
                        self.visit_fstring_elements(
                            &part.elements,
                            part.flags.prefix().is_raw(),
                            interpolated_body_anchor(self.source, part.range),
                        );
                    }
                }
                _ => {
                    if self.depth < self.depth_limit {
                        self.depth += 1;
                        ast::visitor::walk_expr(self, expr);
                        self.depth -= 1;
                    }
                }
            }
        }
    }

    enum GrammarActionReachability {
        Reached,
        BlockedByTypeComment(TextRange),
        BlockedByUnownedTypeComment(ruff_python_parser::TypeCommentToken),
        BlockedBySingle(TextRange),
        Unavailable(GrammarReachabilityUnavailable),
    }

    impl fmt::Debug for GrammarActionReachability {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Reached => f.write_str("Reached"),
                Self::BlockedByTypeComment(range) => {
                    f.debug_tuple("BlockedByTypeComment").field(range).finish()
                }
                Self::BlockedByUnownedTypeComment(comment) => f
                    .debug_tuple("BlockedByUnownedTypeComment")
                    .field(comment)
                    .finish(),
                Self::BlockedBySingle(range) => {
                    f.debug_tuple("BlockedBySingle").field(range).finish()
                }
                Self::Unavailable(reason) => f.debug_tuple("Unavailable").field(reason).finish(),
            }
        }
    }

    #[derive(Debug)]
    enum GrammarReachabilityUnavailable {
        MissingCommentToken,
        NoncanonicalComment,
        UnboundComment,
        UnboundSingleCompletion,
        IndependentStreams,
        StaleCommentAcceptance,
    }

    /// The first-pass lexer owns the original kind. The parser may consume a
    /// soft keyword as Identifier, retaining exactly its range and empty flags.
    /// This is classification, not lexer recovery or a stale provenance point.
    fn parser_classified_token_matches(
        original: &ast::token::Token,
        classified: &ast::token::Token,
    ) -> bool {
        use ast::token::{Token, TokenFlags, TokenKind};
        use ruff_text_size::Ranged;
        original == classified
            || (original.kind().is_soft_keyword()
                && *original == Token::new(original.kind(), original.range(), TokenFlags::empty())
                && *classified
                    == Token::new(TokenKind::Identifier, original.range(), TokenFlags::empty()))
    }

    fn grammar_action_reachability(
        source_file: &compiler::core::SourceFile,
        parsed: &ruff_python_parser::Parsed<ast::Mod>,
        action: &ruff_python_parser::RelativeLazyImportWarning,
        type_comment_rejection: Option<&crate::stdlib::_ast::TypeCommentRejection>,
        type_comments: bool,
        single_mode: bool,
    ) -> GrammarActionReachability {
        use GrammarActionReachability::{
            BlockedBySingle, BlockedByTypeComment, Reached, Unavailable,
        };
        use GrammarReachabilityUnavailable as UnavailableReason;
        use ruff_text_size::Ranged;
        // The existing validator has intentionally retained broader line
        // heuristics (Unicode trim_start and bare-star/duplicate checks). Its
        // unchanged later error is not evidence of a canonical TYPE_COMMENT
        // barrier when this actual Comment is ordinary trivia or TYPE_IGNORE.
        // Missing identities remain errors; they are never inferred ordinary.
        let type_comment_rejection = type_comment_rejection.filter(|rejection| {
            rejection
                .token
                .and_then(|range| parsed.tokens().iter().find(|token| token.range() == range))
                .is_none_or(|token| {
                    if token.kind() != ast::token::TokenKind::Comment
                        || source_file
                            .source_text()
                            .get(token.start().to_usize()..token.end().to_usize())
                            .is_none_or(|text| !text.starts_with('#'))
                    {
                        return true;
                    }
                    ruff_python_parser::classify_type_comment(source_file.source_text(), token)
                        .is_some_and(|comment| {
                            comment.kind == ruff_python_parser::TypeCommentKind::Regular
                        })
                })
        });
        let reads = parsed.diagnostic_token_reads();
        if !matches!(
            parsed.diagnostic_continuity(),
            ruff_python_parser::DiagnosticContinuity::SingleStream { .. }
        ) {
            return Unavailable(UnavailableReason::IndependentStreams);
        }
        if type_comments {
            // A stale certificate is not an absent owner and cannot suppress a
            // validator rejection. Validate every relevant certificate first.
            for accepted in parsed
                .type_comment_acceptances()
                .iter()
                .filter(|accepted| accepted.comment.token.start() < action.tokenizer_end)
            {
                if !accepted.point.original_lexical_prefix
                    || accepted.point.token != accepted.following
                    || !parsed.tokens().iter().any(|token| {
                        token.range() == accepted.comment.token
                            && ruff_python_parser::classify_type_comment(
                                source_file.source_text(),
                                token,
                            )
                            .is_some_and(|comment| comment == accepted.comment)
                    })
                    || !parsed
                        .tokens()
                        .iter()
                        .any(|token| token == &accepted.preceding)
                    || !parsed
                        .tokens()
                        .iter()
                        .any(|token| parser_classified_token_matches(&accepted.following, token))
                {
                    return Unavailable(UnavailableReason::StaleCommentAcceptance);
                }
            }
            for token in parsed
                .tokens()
                .iter()
                .take_while(|token| token.start() < action.module.start())
            {
                let Some(comment) =
                    ruff_python_parser::classify_type_comment(source_file.source_text(), token)
                        .filter(|comment| {
                            comment.kind == ruff_python_parser::TypeCommentKind::Regular
                        })
                else {
                    continue;
                };
                if !parsed.type_comment_acceptances().iter().any(|accepted| {
                    accepted.comment.token == token.range()
                        && accepted.point.original_lexical_prefix
                        && accepted.point.token == accepted.following
                        && parsed
                            .tokens()
                            .iter()
                            .any(|token| token == &accepted.preceding)
                        && parsed.tokens().iter().any(|token| {
                            parser_classified_token_matches(&accepted.following, token)
                        })
                }) && type_comment_rejection
                    .is_none_or(|rejection| rejection.token != Some(token.range()))
                {
                    if parsed
                        .type_comment_acceptances()
                        .iter()
                        .any(|accepted| accepted.comment.token == token.range())
                    {
                        return Unavailable(UnavailableReason::StaleCommentAcceptance);
                    }
                    // All six successful canonical TYPE_COMMENT owner families
                    // are positively instrumented, including the actual async
                    // with-item branch. An earlier original regular token with
                    // no owner is a grammar barrier, not validator acceptance.
                    let original = parsed
                        .diagnostic_lexer_snapshot()
                        .is_some_and(|snapshot| token.end() <= snapshot.seed().token.start())
                        || reads.iter().any(|read| {
                            read.token == *token
                                && read.origin
                                    == ruff_python_parser::DiagnosticReadOrigin::FirstPass
                                && read.type_comment.is_some()
                        });
                    if original {
                        return GrammarActionReachability::BlockedByUnownedTypeComment(comment);
                    }
                    return Unavailable(UnavailableReason::UnboundComment);
                }
            }
        }
        if let Some(rejection) = type_comment_rejection
            // A later validation error cannot undo an already completed action.
            && rejection.token.is_none_or(|range| range.start() < action.tokenizer_end)
            && !parsed.type_comment_acceptances().iter().any(|accepted| {
                rejection.token == Some(accepted.comment.token)
            })
        {
            let Some(range) = rejection.token else {
                return Unavailable(UnavailableReason::MissingCommentToken);
            };
            let token = parsed.tokens().iter().find(|token| token.range() == range);
            let Some(token) = token else {
                return Unavailable(UnavailableReason::MissingCommentToken);
            };
            if !ruff_python_parser::classify_type_comment(source_file.source_text(), token)
                .is_some_and(|comment| comment.kind == ruff_python_parser::TypeCommentKind::Regular)
            {
                return Unavailable(UnavailableReason::NoncanonicalComment);
            }
            if let Some(read) = reads.iter().find(|read| read.token.range() == range) {
                if read.origin != ruff_python_parser::DiagnosticReadOrigin::FirstPass
                    || read.type_comment.is_none()
                {
                    return Unavailable(UnavailableReason::UnboundComment);
                }
                if read.ordinal <= action.tokenizer_read {
                    let completed_bare_lookahead = read.ordinal == action.tokenizer_read
                        && action
                            .type_comment_lookahead
                            .is_some_and(|comment| comment.token == range)
                        && rejection.rule
                            == crate::stdlib::_ast::TypeCommentRejectionRule::StatementPosition;
                    if !completed_bare_lookahead {
                        return BlockedByTypeComment(range);
                    }
                }
            } else if parsed.diagnostic_lexer_snapshot().is_some_and(|snapshot| {
                // The producer captures only an eligible error-free original
                // header. Earlier Comment tokens are before the lazy journal,
                // not missing recovered suffix records. Their actual token
                // order, not the validator's longer diagnostic span, blocks it.
                range.end() <= snapshot.seed().token.start() && range.end() <= action.module.start()
            }) {
                return BlockedByTypeComment(range);
            } else {
                return Unavailable(UnavailableReason::UnboundComment);
            }
        }
        if !single_mode {
            return Reached;
        }
        match compiler::single_mode_statement_boundary(parsed) {
            Some(compiler::SingleModeStatementBoundary::Simple { newline_index, end }) => {
                if action.module.start() < end {
                    return Reached;
                }
                let original_newline = parsed
                    .diagnostic_parser_progress()
                    .and_then(|progress| progress.first_logical_newline.as_ref());
                let Some(completion) = original_newline else {
                    return Unavailable(UnavailableReason::UnboundSingleCompletion);
                };
                if !completion.parser_errors_absent
                    || !completion.point.original_lexical_prefix
                    || parsed.tokens().get(newline_index) != Some(&completion.point.token)
                {
                    return Unavailable(UnavailableReason::UnboundSingleCompletion);
                }
                // Physical NEWLINE consumption proves Simple completion. Ruff's
                // synthetic EOF newline still requires the canonical Single
                // control and is not accepted merely because its span is empty.
                if completion.point.token.range().is_empty() {
                    return Unavailable(UnavailableReason::UnboundSingleCompletion);
                }
                BlockedBySingle(completion.point.token.range())
            }
            Some(compiler::SingleModeStatementBoundary::Compound { end }) => {
                if action.module.start() < end {
                    return Reached;
                }
                let Some(completion) = parsed
                    .diagnostic_parser_progress()
                    .and_then(|progress| progress.first_statement_return.as_ref())
                else {
                    return Unavailable(UnavailableReason::UnboundSingleCompletion);
                };
                let token = &completion.point.token;
                if completion.kind != ruff_python_parser::DiagnosticStatementKind::Compound
                    || completion.statement.end() != end
                    || !completion.parser_errors_absent
                    || !completion.point.original_lexical_prefix
                    || token.range().is_empty()
                    || matches!(
                        token.kind(),
                        ast::token::TokenKind::EndOfFile
                            | ast::token::TokenKind::Newline
                            | ast::token::TokenKind::Indent
                            | ast::token::TokenKind::Dedent
                            | ast::token::TokenKind::Unknown
                    )
                    || !parsed
                        .tokens()
                        .iter()
                        .any(|candidate| parser_classified_token_matches(token, candidate))
                    || action.module.start() < token.start()
                {
                    return Unavailable(UnavailableReason::UnboundSingleCompletion);
                }
                // The clean first compound has returned at this actual next
                // syntax token. Single rejects it before parsing another stmt.
                BlockedBySingle(token.range())
            }
            None => Unavailable(UnavailableReason::UnboundSingleCompletion),
        }
    }

    /// Preflighted tokenizer-only suffix after a proven completed grammar action.
    /// Build this before invoking warning callbacks; unavailable provenance must
    /// never be discovered only after partially emitting a candidate batch.
    struct GrammarRecoveryPlan {
        events: alloc::sync::Arc<GrammarRecoveryEvents>,
        frontier: usize,
        current_error_line: usize,
    }

    struct GrammarRecoveryEvents {
        reads: Vec<(usize, ast::token::Token)>,
        warnings: Vec<(usize, ruff_python_parser::LexicalWarningDiscovery)>,
        stop: Option<ruff_python_parser::LexicalDiscovery>,
    }

    enum GrammarRecoveryUnavailable {
        SourceOrMode,
        Frontier,
        Binding(ruff_python_parser::lexer::DiagnosticForkBindingError),
        Unclassified(ruff_python_parser::LexicalDiscovery),
        MissingRead,
    }

    impl fmt::Debug for GrammarRecoveryUnavailable {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::SourceOrMode => f.write_str("SourceOrMode"),
                Self::Frontier => f.write_str("Frontier"),
                Self::Binding(error) => f.debug_tuple("Binding").field(error).finish(),
                Self::Unclassified(discovery) => {
                    f.debug_tuple("Unclassified").field(discovery).finish()
                }
                Self::MissingRead => f.write_str("MissingRead"),
            }
        }
    }

    /// Barry preparation preserves byte positions and changes only '<>' to '!='.
    /// Bind that explicit map, not a source length or diagnostic-offset guess.
    fn same_barry_source_positions(original: &str, prepared: &str) -> bool {
        if original.len() != prepared.len() {
            return false;
        }
        let (original, prepared) = (original.as_bytes(), prepared.as_bytes());
        let mut cursor = 0;
        while cursor < original.len() {
            if original[cursor] == prepared[cursor] {
                cursor += 1;
            } else if original.get(cursor..cursor + 2) == Some(b"<>")
                && prepared.get(cursor..cursor + 2) == Some(b"!=")
            {
                cursor += 2;
            } else {
                return false;
            }
        }
        true
    }

    fn prepare_grammar_recovery(
        source: &str,
        prepared_source: &compiler::BarrySource<'_>,
        parsed: &ruff_python_parser::Parsed<ast::Mod>,
        actions: &[ruff_python_parser::RelativeLazyImportWarning],
        index: &SourceDiagnosticIndex,
        prompt: bool,
    ) -> Result<
        (
            alloc::sync::Arc<GrammarRecoveryEvents>,
            Vec<Option<GrammarRecoveryPlan>>,
        ),
        GrammarRecoveryUnavailable,
    > {
        use ruff_python_parser::lexer::DiagnosticForkTermination;
        let Some(last) = actions.iter().max_by_key(|action| action.tokenizer_read) else {
            return Err(GrammarRecoveryUnavailable::Frontier);
        };
        let snapshot = parsed
            .diagnostic_lexer_snapshot()
            .ok_or(GrammarRecoveryUnavailable::Frontier)?;
        if snapshot.mode() != ruff_python_parser::Mode::Module
            || snapshot.source() != prepared_source.source()
            || !same_barry_source_positions(source, snapshot.source())
        {
            return Err(GrammarRecoveryUnavailable::SourceOrMode);
        }
        let reads = parsed.diagnostic_token_reads();
        for action in actions {
            let filled = reads
                .get(action.tokenizer_read)
                .filter(|read| read.ordinal == action.tokenizer_read)
                .and_then(|read| read.read_range)
                .ok_or(GrammarRecoveryUnavailable::Frontier)?;
            if filled.start() != action.tokenizer_start || filled.end() != action.tokenizer_end {
                return Err(GrammarRecoveryUnavailable::Frontier);
            }
        }
        if prompt {
            // The real prompt does not recover the remaining source after a
            // warning error (pegen_errors.c). Retain only observed original
            // read identities for ordering; never resume or drain a fork here.
            if !matches!(
                parsed.diagnostic_continuity(),
                ruff_python_parser::DiagnosticContinuity::SingleStream { .. }
            ) || reads[..=last.tokenizer_read]
                .iter()
                .any(|read| read.origin != ruff_python_parser::DiagnosticReadOrigin::FirstPass)
            {
                return Err(GrammarRecoveryUnavailable::Frontier);
            }
            // A parser error alone does not rewrite the lexer. The producer's
            // existing-read view certifies only the observed original lineage,
            // stopping before an actual divergence or at the first hard stop.
            // This is ordering metadata, never a prompt suffix recovery pass.
            let continuation = ruff_python_parser::original_continuation_after(
                reads,
                parsed.diagnostic_continuity(),
                last.tokenizer_read,
            );
            use ruff_python_parser::ContinuationTermination;
            match &continuation.termination {
                ContinuationTermination::IndependentStreams
                | ContinuationTermination::UnboundFrontier { .. }
                | ContinuationTermination::FailureAlreadyRead { .. } => {
                    return Err(GrammarRecoveryUnavailable::Frontier);
                }
                ContinuationTermination::UnclassifiedDiscovery { discovery, .. } => {
                    return Err(GrammarRecoveryUnavailable::Unclassified(
                        (*discovery).clone(),
                    ));
                }
                ContinuationTermination::MissingRead { ordinal } if *ordinal < reads.len() => {
                    return Err(GrammarRecoveryUnavailable::MissingRead);
                }
                ContinuationTermination::Diverged(divergence)
                    if divergence.next_read <= last.tokenizer_read =>
                {
                    return Err(GrammarRecoveryUnavailable::Frontier);
                }
                _ => {}
            }
            let mut anchors = Vec::new();
            let mut warnings = Vec::new();
            for read in reads[..=last.tokenizer_read]
                .iter()
                .chain(continuation.reads)
            {
                anchors.push((read.ordinal, read.token));
                let first_stop = read.discoveries.iter().find(|discovery| {
                    !discovery.eof_cleanup
                        && discovery.disposition
                            != ruff_python_parser::LexicalDisposition::ParserToken
                });
                warnings.extend(
                    read.warnings
                        .iter()
                        .filter(|warning| {
                            first_stop.is_none_or(|stop| warning.cursor <= stop.cursor)
                        })
                        .cloned()
                        .map(|warning| (read.ordinal, warning)),
                );
            }
            // Truncation is not treated as EOF: the distinct prompt wrapper has
            // no recovery plans and never consults a suffix terminal outcome.
            let stop = match continuation.termination {
                ContinuationTermination::HardStop { discovery, .. } => Some(discovery.clone()),
                _ => None,
            };
            let events = alloc::sync::Arc::new(GrammarRecoveryEvents {
                reads: anchors,
                warnings,
                stop,
            });
            return Ok((events, actions.iter().map(|_| None).collect()));
        }
        // Bind the complete original prefix once. Every earlier action then
        // shares these exact reads and one independent tokenizer-only suffix.
        let mut fork = snapshot
            .resume_after(reads, parsed.diagnostic_continuity(), last.tokenizer_read)
            .map_err(GrammarRecoveryUnavailable::Binding)?;
        let mut anchors = Vec::new();
        let mut warnings = Vec::new();
        for read in &reads[..=last.tokenizer_read] {
            anchors.push((read.ordinal, read.token));
            warnings.extend(
                read.warnings
                    .iter()
                    .cloned()
                    .map(|warning| (read.ordinal, warning)),
            );
        }
        while let Some(read) = fork.next_read() {
            anchors.push((read.ordinal, read.token));
            let first_stop = read.discoveries.iter().find(|discovery| {
                !discovery.eof_cleanup
                    && discovery.disposition != ruff_python_parser::LexicalDisposition::ParserToken
            });
            // Original warning hooks run before a later same-read failure.
            // A brace lookahead can extend beyond the returned MIDDLE range.
            warnings.extend(
                read.warnings
                    .into_iter()
                    .filter(|warning| first_stop.is_none_or(|stop| warning.cursor <= stop.cursor))
                    .map(|warning| (read.ordinal, warning)),
            );
        }
        let stop = match fork.termination() {
            Some(DiagnosticForkTermination::EndOfInput { .. }) => None,
            Some(DiagnosticForkTermination::HardStop { discovery, .. }) => Some(discovery.clone()),
            Some(DiagnosticForkTermination::UnclassifiedDiscovery { discovery, .. }) => {
                return Err(GrammarRecoveryUnavailable::Unclassified(discovery.clone()));
            }
            Some(DiagnosticForkTermination::MissingRead { .. }) | None => {
                return Err(GrammarRecoveryUnavailable::MissingRead);
            }
        };
        let events = alloc::sync::Arc::new(GrammarRecoveryEvents {
            reads: anchors,
            warnings,
            stop,
        });
        let plans = actions
            .iter()
            .map(|action| {
                Some(GrammarRecoveryPlan {
                    events: events.clone(),
                    frontier: action.tokenizer_read,
                    // pegen_errors.c uses the START line of the furthest filled token.
                    current_error_line: index
                        .lines
                        .partition_point(|line| line.start <= action.tokenizer_start.to_usize()),
                })
            })
            .collect();
        Ok((events, plans))
    }

    fn relative_lazy_message(action: &ruff_python_parser::RelativeLazyImportWarning) -> String {
        let dots = ".".repeat(action.level as usize);
        format!(
            "'from {dots} lazy import' is the same as 'from {dots}lazy import'; \
                 did you mean 'lazy from {dots} import'?"
        )
    }

    impl VirtualMachine {
        fn emit_relative_lazy_warning(
            &self,
            source: &str,
            filename: &str,
            original_filename: Option<&Py<PyStr>>,
            module: Option<&PyObject>,
            action: &ruff_python_parser::RelativeLazyImportWarning,
            index: &SourceDiagnosticIndex,
        ) -> Result<(), CompileWarningError> {
            let message = relative_lazy_message(action);
            let offset = action.module.start().to_usize();
            let line = &index.lines[index.lines.partition_point(|line| line.start <= offset) - 1];
            warn_syntax_at_offset(
                source,
                filename,
                original_filename,
                module,
                offset,
                message.clone(),
                self,
            )
            .map_err(|mut error| {
                // Grammar actions use UTF-8 byte columns and text=None,
                // unlike numeric/brace tokenizer character columns.
                error.offset = offset - line.start + 1;
                error.replacement = Some(SyntaxErrorReplacement {
                    message,
                    end_offset: action.module.end().to_usize() - line.start + 1,
                });
                error.omit_source_text = true;
                error
            })
        }

        /// Consume an already preflighted suffix only after a parser-origin
        /// SyntaxWarning became a pending SyntaxError. No parser action is replayed.
        fn emit_grammar_recovery(
            &self,
            context: WarningSource<'_>,
            module: Option<&PyObject>,
            mode: compiler::Mode,
            plan: GrammarRecoveryPlan,
            pending: VmCompileError,
            index: &SourceDiagnosticIndex,
        ) -> VmCompileError {
            let WarningSource {
                source,
                filename,
                original_filename,
            } = context;
            use ruff_python_parser::{LexicalDisposition, LexicalWarningKind};
            for (_, warning) in plan
                .events
                .warnings
                .iter()
                .filter(|(read, _)| *read > plan.frontier)
            {
                let result = match warning.kind {
                    LexicalWarningKind::Numeric(kind) => warn_numeric_literal(
                        source,
                        filename,
                        original_filename,
                        module,
                        warning.last_offset.to_usize(),
                        format!("invalid {kind} literal"),
                        self,
                    ),
                    LexicalWarningKind::InterpolatedBrace(ch) => warn_invalid_escape_sequence_at(
                        source,
                        InvalidEscape::Char {
                            ch,
                            offset: warning.offset.to_usize(),
                        },
                        filename,
                        original_filename,
                        module,
                        Some(tokenizer_brace_location(
                            index,
                            warning.offset.to_usize(),
                            warning.cursor.to_usize(),
                        )),
                        self,
                    ),
                };
                if let Err(error) = result {
                    // pegen_errors.c restores the pending parser exception if
                    // tokenizer recovery fails inside any active f/t mode.
                    return if warning.context.in_interpolated_string {
                        pending
                    } else {
                        VmCompileError::Warning(error)
                    };
                }
            }
            let Some(stop) = plan.events.stop.as_ref() else {
                return pending;
            };
            if stop.context.in_interpolated_string {
                return pending;
            }
            let parse_error = match stop.disposition {
                LexicalDisposition::RaisedErrorToken => ruff_python_parser::ParseError {
                    error: ruff_python_parser::ParseErrorType::Lexical(stop.error.clone()),
                    location: stop.diagnostic,
                },
                LexicalDisposition::BareErrorToken => {
                    let Some(opener) = stop.context.open_bracket else {
                        return pending;
                    };
                    let opener_line = index
                        .lines
                        .partition_point(|line| line.start <= opener.offset.to_usize());
                    if plan.current_error_line <= opener_line {
                        return pending;
                    }
                    ruff_python_parser::ParseError {
                        error: ruff_python_parser::ParseErrorType::Lexical(
                            ruff_python_parser::LexicalErrorType::UnclosedBracket {
                                opening: opener.kind,
                                incomplete: false,
                            },
                        ),
                        location: TextRange::empty(opener.offset),
                    }
                }
                LexicalDisposition::ParserToken | LexicalDisposition::DecoderOrUnclassified => {
                    // Preflight never accepts these as a terminal stop.
                    unreachable!("grammar recovery plan only contains classified tokenizer stops")
                }
            };
            let source_file = compiler::core::SourceFileBuilder::new(filename, source).finish();
            let mut error = CompileError::from_ruff_parse_error(parse_error, &source_file, mode);
            if let CompileError::Parse(error) = &mut error {
                // Match main's existing after-syntax-error recovery contract:
                // a later unterminated string cannot request more input.
                error.is_unclosed_string = false;
            }
            VmCompileError::Compile(error)
        }
    }

    struct CollectedParserWarnings {
        warnings: Vec<ParserWarning>,
        tokenizer_recovery: bool,
        has_valid_syntax: bool,
        single_boundary: Option<compiler::SingleModeStatementBoundary>,
        type_comment_error_offset: Option<usize>,
        grammar_type_comment_error: Option<CompileError>,
    }

    enum GrammarCompositionUnavailable {
        Reachability {
            module: TextRange,
            reason: GrammarReachabilityUnavailable,
        },
        Recovery(GrammarRecoveryUnavailable),
        UnboundWarning {
            offset: usize,
            tokenizer: bool,
        },
        MissingRecoveryPlan,
    }

    impl fmt::Debug for GrammarCompositionUnavailable {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Reachability { module, reason } => f
                    .debug_struct("Reachability")
                    .field("module", module)
                    .field("reason", reason)
                    .finish(),
                Self::Recovery(error) => f.debug_tuple("Recovery").field(error).finish(),
                Self::UnboundWarning { offset, tokenizer } => f
                    .debug_struct("UnboundWarning")
                    .field("offset", offset)
                    .field("tokenizer", tokenizer)
                    .finish(),
                Self::MissingRecoveryPlan => f.write_str("MissingRecoveryPlan"),
            }
        }
    }

    struct GrammarCollection {
        mode: compiler::Mode,
        prompt: bool,
        status: Result<bool, GrammarCompositionUnavailable>,
    }

    /// A same-source, same-mode batch prepared before warning callbacks.
    /// Provenance failures indicate an internal construction invariant violation;
    /// they must never silently drop an action or enter a fallback warning path.
    struct PreparedGrammarWarnings {
        source: Box<str>,
        filename: String,
        collected: CollectedParserWarnings,
    }

    fn prepare_relative_lazy_parser_warnings(
        source: &str,
        filename: &str,
        mode: compiler::Mode,
        inherited_barry_as_flufl: bool,
        type_comments: bool,
    ) -> Result<Option<PreparedGrammarWarnings>, GrammarCompositionUnavailable> {
        let parser_mode = match mode {
            compiler::Mode::Eval => ruff_python_parser::Mode::Expression,
            compiler::Mode::Exec | compiler::Mode::Single | compiler::Mode::BlockExpr => {
                ruff_python_parser::Mode::Module
            }
        };
        let mut grammar = GrammarCollection {
            mode,
            prompt: false,
            status: Ok(false),
        };
        let collected = collect_parser_warnings_impl(
            WarningSource::new(source, filename),
            Some(parser_mode),
            inherited_barry_as_flufl,
            InterpolatedBodyDiagnostics::TokenAnchored {
                implicit_final_newline: matches!(mode, compiler::Mode::Exec),
            },
            matches!(mode, compiler::Mode::Single),
            type_comments,
            Some(&mut grammar),
        );
        Ok(grammar.status?.then_some(PreparedGrammarWarnings {
            source: source.into(),
            filename: filename.to_owned(),
            collected,
        }))
    }

    /// Distinct ownership prevents a no-recovery prompt batch being passed to
    /// the standard emitter by mistake.
    struct PreparedPromptGrammarWarnings(PreparedGrammarWarnings);

    fn prepare_interactive_relative_lazy_warnings(
        source: &str,
        filename: &str,
        inherited_barry_as_flufl: bool,
    ) -> Result<Option<PreparedPromptGrammarWarnings>, GrammarCompositionUnavailable> {
        let mut grammar = GrammarCollection {
            mode: compiler::Mode::Single,
            prompt: true,
            status: Ok(false),
        };
        let collected = collect_parser_warnings_impl(
            WarningSource::new(source, filename),
            Some(ruff_python_parser::Mode::Module),
            inherited_barry_as_flufl,
            InterpolatedBodyDiagnostics::Legacy,
            false,
            false,
            Some(&mut grammar),
        );
        Ok(grammar
            .status?
            .then_some(PreparedPromptGrammarWarnings(PreparedGrammarWarnings {
                source: source.into(),
                filename: filename.to_owned(),
                collected,
            })))
    }

    fn append_relative_lazy_warnings(
        source_file: &compiler::core::SourceFile,
        prepared_source: &compiler::BarrySource<'_>,
        parsed: &ruff_python_parser::Parsed<ast::Mod>,
        mode: compiler::Mode,
        type_comments: bool,
        prompt: bool,
        warnings: &mut Vec<ParserWarning>,
    ) -> Result<(bool, Option<(usize, CompileError)>), GrammarCompositionUnavailable> {
        use ruff_text_size::Ranged;
        if parsed.relative_lazy_import_warnings().is_empty() {
            return Ok((false, None));
        }
        let rejection = type_comments
            .then(|| crate::stdlib::_ast::unfiltered_type_comment_rejection(source_file, parsed))
            .flatten();
        let mut actions = Vec::new();
        let mut unowned_comment = None;
        for action in parsed.relative_lazy_import_warnings() {
            match grammar_action_reachability(
                source_file,
                parsed,
                action,
                rejection.as_ref(),
                type_comments,
                matches!(mode, compiler::Mode::Single),
            ) {
                GrammarActionReachability::Reached => actions.push(action.clone()),
                GrammarActionReachability::BlockedByUnownedTypeComment(comment) => {
                    if unowned_comment.is_none() {
                        unowned_comment = Some(comment);
                    }
                }
                GrammarActionReachability::BlockedByTypeComment(_)
                | GrammarActionReachability::BlockedBySingle(_) => {}
                GrammarActionReachability::Unavailable(reason) => {
                    return Err(GrammarCompositionUnavailable::Reachability {
                        module: action.module,
                        reason,
                    });
                }
            }
        }
        let barrier = if let Some(comment) = unowned_comment.filter(|comment| {
            // Retain only rejection holes; the existing validator keeps ownership
            // and priority whenever it already has a typed rejection.
            rejection.is_none()
                && (!matches!(mode, compiler::Mode::Single)
                    || crate::stdlib::_ast::type_comment_precedes_single_boundary(
                        comment.token.start().to_usize(),
                        source_file,
                        parsed,
                    ))
        }) {
            let diagnostic = if let Some(parameter) = parsed
                .diagnostic_parser_progress()
                .and_then(|progress| progress.parameter_type_comment_barrier.as_ref())
                .filter(|parameter| parameter.comment == comment)
            {
                if !parameter.point.original_lexical_prefix
                    || parameter.point.token.kind() != ast::token::TokenKind::Comma
                    || !parsed
                        .tokens()
                        .iter()
                        .any(|token| token == &parameter.point.token)
                    || !parsed
                        .tokens()
                        .iter()
                        .any(|token| token == &parameter.preceding)
                    || parameter.preceding.end() > comment.token.start()
                    || comment.token.end() > parameter.point.token.start()
                {
                    return Err(GrammarCompositionUnavailable::Reachability {
                        module: comment.token,
                        reason: GrammarReachabilityUnavailable::StaleCommentAcceptance,
                    });
                }
                parameter.point.token.range()
            } else {
                comment.payload
            };
            Some((
                comment.token.start().to_usize(),
                crate::stdlib::_ast::type_comment_parse_error(
                    source_file,
                    "invalid syntax",
                    diagnostic.start().to_usize(),
                    diagnostic.end().to_usize(),
                ),
            ))
        } else {
            None
        };
        if actions.is_empty() {
            return Ok((false, barrier));
        }
        actions.sort_by_key(|action| action.tokenizer_read);
        let source = source_file.source_text();
        let index = alloc::sync::Arc::new(SourceDiagnosticIndex::new(source));
        let (events, recovery) =
            prepare_grammar_recovery(source, prepared_source, parsed, &actions, &index, prompt)
                .map_err(GrammarCompositionUnavailable::Recovery)?;
        let seed_start = events.reads[0].1.start().to_usize();
        // Every relative action is placed by its filled read identity, not a
        // byte-offset tie. In '*1and' or '(x)1and', the number starts exactly
        // where the completed target ends, but belongs to the NEXT read.
        let mut insertions = Vec::with_capacity(warnings.len());
        let mut replacements: Vec<Option<ParserWarning>> =
            (0..warnings.len()).map(|_| None).collect();
        for (warning_index, warning) in warnings.iter().enumerate() {
            if let ParserWarning::Escape(escape, EscapeOrigin::Parser, ..) = warning
                && parsed
                    .invalid_import_alias_decoders()
                    .iter()
                    .any(|invalid| {
                        invalid.first.original_lexical_prefix
                            && invalid
                                .first
                                .read
                                .and_then(|ordinal| parsed.diagnostic_token_reads().get(ordinal))
                                .is_some_and(|read| {
                                    read.token == invalid.first.token
                                        && read.origin
                                            == ruff_python_parser::DiagnosticReadOrigin::FirstPass
                                })
                            && parsed.tokens().iter().any(|token| {
                                token.kind() == ast::token::TokenKind::As
                                    && token.range() == invalid.as_keyword
                            })
                            && actions.iter().any(|action| {
                                action.module.end() <= invalid.imported_name.start()
                                    && invalid.imported_name.end() <= action.targets_end
                            })
                            && invalid.expression.start().to_usize() <= escape.offset()
                            && escape.offset() < invalid.expression.end().to_usize()
                    })
            {
                // Only this actual invalid-alias production is pass-two-only.
                // Keep its tokenizer warnings and every proven grammar action.
                insertions.push(None);
                continue;
            }
            if warning.emission_offset() < seed_start {
                insertions.push(Some(0));
                continue;
            }
            if matches!(warning, ParserWarning::Escape(_, EscapeOrigin::Parser, ..)) {
                // Preserve the existing non-grammar emission order. An import action cannot
                // occur within a literal/interpolation; the exact invalid-alias
                // decoder production was excluded above. Its original escape
                // offset therefore separates it from every surrounding action.
                let before = actions
                    .partition_point(|action| action.module.start().to_usize() < warning.offset());
                insertions.push(Some(before * 2));
                continue;
            }
            let (read, tokenizer) = match warning {
                ParserWarning::Numeric {
                    offset,
                    last_offset,
                    ..
                } => (
                    events
                        .warnings
                        .iter()
                        .find(|(_, discovery)| {
                            matches!(
                                discovery.kind,
                                ruff_python_parser::LexicalWarningKind::Numeric(_)
                            ) && discovery.offset.to_usize() == *offset
                                && discovery.last_offset.to_usize() == *last_offset
                        })
                        .map(|(read, _)| *read),
                    true,
                ),
                ParserWarning::Escape(escape, EscapeOrigin::Tokenizer, ..) => (
                    events
                        .warnings
                        .iter()
                        .find(|(_, discovery)| {
                            matches!(
                                discovery.kind,
                                ruff_python_parser::LexicalWarningKind::InterpolatedBrace(_)
                            ) && discovery.offset.to_usize() == escape.offset()
                        })
                        .map(|(read, _)| *read),
                    true,
                ),
                ParserWarning::Escape(_, EscapeOrigin::Parser, ..) => {
                    unreachable!("decoder warning was placed between surrounding actions")
                }
                ParserWarning::RelativeLazy { .. } => {
                    unreachable!("actions are appended after preflight")
                }
            };
            let Some(read) = read else {
                if let ParserWarning::Numeric {
                    offset,
                    last_offset,
                    ..
                } = warning
                {
                    // The fallback scanner can consume the final dot of an
                    // original Ellipsis as the start of a neighboring float.
                    // Bind only this exact token boundary to the unique real
                    // numeric discovery, never arbitrary overlapping offsets.
                    if let Some(stop) = events.stop.as_ref()
                        && matches!(stop.error, ruff_python_parser::LexicalErrorType::LeadingZerosInDecimalInteger)
                        && stop.disposition == ruff_python_parser::LexicalDisposition::RaisedErrorToken
                        && !stop.eof_cleanup
                        && let Some(failed_range) = stop.token_range
                        && failed_range.end().to_usize().checked_sub(1) == Some(*last_offset)
                        && stop.cursor == failed_range.end()
                        && events.reads.iter().any(|(read, token)| {
                            token.kind() == ast::token::TokenKind::Unknown
                                && token.range() == failed_range
                                && !events.warnings.iter().any(|(warning_read, discovery)| {
                                    warning_read == read
                                        && matches!(discovery.kind, ruff_python_parser::LexicalWarningKind::Numeric(_))
                                })
                                && actions.iter().any(|action| {
                                    parsed.diagnostic_token_reads().get(action.tokenizer_read)
                                        .is_some_and(|filled| {
                                            filled.ordinal == action.tokenizer_read
                                                && filled.origin == ruff_python_parser::DiagnosticReadOrigin::FirstPass
                                                && filled.token.kind() == ast::token::TokenKind::Ellipsis
                                                && filled.token.end().to_usize().checked_sub(1) == Some(*offset)
                                                && failed_range.start() == filled.token.end()
                                                && *read == action.tokenizer_read + 1
                                        })
                                })
                        })
                    {
                        // A leading-zero integer stops before the original
                        // numeric warning hook. The scanner's leading-dot float
                        // is not that token. Retain the exact hard error, not
                        // its false warning alias; prompt still never recovers.
                        insertions.push(None);
                        continue;
                    }
                    let mut adjacent = events.warnings.iter().filter(|(read, discovery)| {
                        matches!(
                            discovery.kind,
                            ruff_python_parser::LexicalWarningKind::Numeric(_)
                        ) && discovery.last_offset.to_usize() == *last_offset
                            && events.reads.iter().any(|(ordinal, token)| {
                                ordinal == read
                                    && token.start() == discovery.offset
                                    && matches!(
                                        token.kind(),
                                        ast::token::TokenKind::Int
                                            | ast::token::TokenKind::Float
                                            | ast::token::TokenKind::Complex
                                    )
                                    && *last_offset < token.end().to_usize()
                            })
                            && actions.iter().any(|action| {
                                parsed
                                    .diagnostic_token_reads()
                                    .get(action.tokenizer_read)
                                    .is_some_and(|filled| {
                                        filled.ordinal == action.tokenizer_read
                                        && filled.origin
                                            == ruff_python_parser::DiagnosticReadOrigin::FirstPass
                                        && filled.token.kind() == ast::token::TokenKind::Ellipsis
                                        && filled.token.end().to_usize().checked_sub(1)
                                            == Some(*offset)
                                        && *last_offset >= filled.token.end().to_usize()
                                        && discovery.offset == filled.token.end()
                                        && *read > action.tokenizer_read
                                    })
                            })
                    });
                    if let Some((read, discovery)) = adjacent.next()
                        && adjacent.next().is_none()
                        && let ruff_python_parser::LexicalWarningKind::Numeric(kind) =
                            discovery.kind
                    {
                        if warnings.iter().any(|candidate| {
                            matches!(candidate, ParserWarning::Numeric { offset, last_offset, .. }
                                if *offset == discovery.offset.to_usize()
                                    && *last_offset == discovery.last_offset.to_usize())
                        }) {
                            // An already exact event remains independently
                            // bound below; do not emit its scanner alias twice.
                            insertions.push(None);
                            continue;
                        }
                        replacements[warning_index] = Some(ParserWarning::Numeric {
                            offset: discovery.offset.to_usize(),
                            last_offset: discovery.last_offset.to_usize(),
                            message: format!("invalid {kind} literal"),
                            in_interpolation: discovery.context.in_interpolated_string,
                        });
                        let before =
                            actions.partition_point(|action| action.tokenizer_read < *read);
                        insertions.push(Some(before * 2));
                        continue;
                    }
                }
                let non_code_span = match warning {
                    ParserWarning::Numeric {
                        offset,
                        last_offset,
                        ..
                    } if offset <= last_offset => Some((*offset, *last_offset)),
                    ParserWarning::Escape(
                        InvalidEscape::Char {
                            ch: '{' | '}',
                            offset,
                        },
                        EscapeOrigin::Tokenizer,
                        ..,
                    ) => offset.checked_add(1).map(|last| (*offset, last)),
                    _ => None,
                };
                if let Some((offset, last_offset)) = non_code_span
                    && actions.iter().any(|action| {
                        parsed.diagnostic_token_reads().get(action.tokenizer_read)
                            .is_some_and(|filled| {
                                filled.ordinal == action.tokenizer_read
                                    && filled.origin == ruff_python_parser::DiagnosticReadOrigin::FirstPass
                                    && matches!(filled.token.kind(),
                                        ast::token::TokenKind::String | ast::token::TokenKind::Comment)
                                    && filled.token.start().to_usize() <= offset
                                    && last_offset < filled.token.end().to_usize()
                                    && !filled.warnings.iter().any(|discovery| {
                                        match (warning, discovery.kind) {
                                            (ParserWarning::Numeric { .. }, ruff_python_parser::LexicalWarningKind::Numeric(_)) => {
                                                discovery.offset.to_usize() == offset
                                                    && discovery.last_offset.to_usize() == last_offset
                                            }
                                            (ParserWarning::Escape(_, EscapeOrigin::Tokenizer, ..),
                                                ruff_python_parser::LexicalWarningKind::InterpolatedBrace(_)) => {
                                                discovery.offset.to_usize() == offset
                                            }
                                            _ => false,
                                        }
                                    })
                            })
                    })
                {
                    // Legacy fallback scanners can lose quote nesting inside
                    // f/t expressions. The exact original STRING/Comment read
                    // proves this wholly contained number or brace is non-code
                    // content, never a tokenizer warning. Do not generalize to
                    // MIDDLE/END, partial spans, parser decoders, or bound events.
                    insertions.push(None);
                    continue;
                }
                let offset = warning.offset();
                if actions.iter().all(|action| {
                    offset < action.tokenizer_start.to_usize()
                        || action.tokenizer_end.to_usize() <= offset
                }) {
                    // Inherited scanners can report warnings after original
                    // lexical stopping. If separated from EVERY filled read,
                    // source order still determines the surrounding actions.
                    // This is placement only, not certified tokenizer reachability.
                    let before =
                        actions.partition_point(|action| action.tokenizer_end.to_usize() <= offset);
                    insertions.push(Some(before * 2));
                    continue;
                }
                return Err(GrammarCompositionUnavailable::UnboundWarning {
                    offset: warning.offset(),
                    tokenizer,
                });
            };
            let before = actions.partition_point(|action| {
                if tokenizer {
                    action.tokenizer_read < read
                } else {
                    action.tokenizer_read <= read
                }
            });
            insertions.push(Some(before * 2));
        }
        // No callbacks or warning-vector mutations preceded successful binding.
        // Keep #9314's stable relative ordering among every non-grammar warning.
        let mut ordered: Vec<_> = core::mem::take(warnings)
            .into_iter()
            .zip(insertions)
            .zip(replacements)
            .filter_map(|((warning, insertion), replacement)| {
                insertion.map(|insertion| (insertion, replacement.unwrap_or(warning)))
            })
            .collect();
        ordered.sort_by_key(|(insertion, warning)| (*insertion, warning.emission_offset()));
        ordered.extend(actions.into_iter().zip(recovery).enumerate().map(
            |(index_in_batch, (action, recovery))| {
                (
                    index_in_batch * 2 + 1,
                    ParserWarning::RelativeLazy {
                        action,
                        recovery,
                        index: index.clone(),
                        mode,
                    },
                )
            },
        ));
        ordered.sort_by_key(|(insertion, _)| *insertion);
        warnings.extend(ordered.into_iter().map(|(_, warning)| warning));
        Ok((true, barrier))
    }

    fn collect_parser_warnings(
        source: &str,
        filename: &str,
        mode: Option<ruff_python_parser::Mode>,
        inherited_barry_as_flufl: bool,
        interpolated_body_diagnostics: InterpolatedBodyDiagnostics,
        single_mode: bool,
        type_comments: bool,
    ) -> CollectedParserWarnings {
        collect_parser_warnings_impl(
            WarningSource::new(source, filename),
            mode,
            inherited_barry_as_flufl,
            interpolated_body_diagnostics,
            single_mode,
            type_comments,
            None,
        )
    }

    fn collect_parser_warnings_impl(
        context: WarningSource<'_>,
        mode: Option<ruff_python_parser::Mode>,
        inherited_barry_as_flufl: bool,
        interpolated_body_diagnostics: InterpolatedBodyDiagnostics,
        single_mode: bool,
        type_comments: bool,
        grammar: Option<&mut GrammarCollection>,
    ) -> CollectedParserWarnings {
        let WarningSource {
            source, filename, ..
        } = context;
        let mut grammar_sorted = false;
        let mut ast_tokenizer_recovery = true;
        let mut has_valid_syntax = false;
        let mut single_boundary = None;
        let mut type_comment_error_offset = None;
        let mut grammar_type_comment_error = None;
        let mut warnings = Vec::new();
        collect_numeric_literal_warnings(source, &mut warnings);
        // The compile that follows rejects this source; parsing it here
        // would build a tree that exhausts the stack when dropped.
        let source_file = compiler::core::SourceFileBuilder::new(filename, source).finish();
        if compiler::pre_parse_source_error(&source_file).is_ok()
            && let Some(mode) = mode
        {
            let options = ruff_python_parser::ParseOptions::from(mode)
                .with_diagnostic_type_comments(grammar.is_some() && type_comments);
            let barry_source = compiler::prepare_barry_as_flufl_source(
                source,
                options.clone(),
                inherited_barry_as_flufl,
            );
            let parsed = ruff_python_parser::parse_unchecked(barry_source.source(), options);
            if parsed.has_valid_syntax() {
                has_valid_syntax = true;
                if single_mode {
                    single_boundary = compiler::single_mode_statement_boundary(&parsed);
                }
                if type_comments {
                    type_comment_error_offset =
                        crate::stdlib::_ast::invalid_type_comment_warning_boundary(
                            &source_file,
                            &parsed,
                            single_mode,
                        );
                }
                if matches!(
                    interpolated_body_diagnostics,
                    InterpolatedBodyDiagnostics::TokenAnchored { .. }
                ) {
                    // AST literal ranges include numbers inside interpolations
                    // without mistaking numeric-looking string contents for
                    // tokens. The text scanner remains the invalid-source path.
                    warnings.clear();
                }
                let mut visitor = EscapeWarningVisitor {
                    source,
                    warnings: &mut warnings,
                    ast_only: true,
                    interpolated_body_diagnostics,
                    diagnostic_index: core::cell::OnceCell::new(),
                    interpolation_depth: 0,
                    depth: 0,
                    depth_limit: compiler::CompileOpts::default().recursion_limit,
                };
                match parsed.syntax() {
                    ast::Mod::Module(module) => {
                        for stmt in &module.body {
                            visitor.visit_stmt(stmt);
                        }
                    }
                    ast::Mod::Expression(expr) => visitor.visit_expr(&expr.body),
                }
            } else {
                ast_tokenizer_recovery = false;
                // The tokenizer can read the token at which parsing fails,
                // but later recovered statements were never parsed by
                // CPython and must not produce string/parser warnings.
                let end = parsed
                    .errors()
                    .iter()
                    // An unclosed-bracket diagnostic points back to its
                    // opening token, not where parsing stopped. Other
                    // errors retain that boundary; otherwise it was EOF.
                    .filter(|error| {
                        !matches!(
                            error.error,
                            ruff_python_parser::ParseErrorType::Lexical(
                                ruff_python_parser::LexicalErrorType::UnclosedBracket { .. }
                            )
                        )
                    })
                    .min_by_key(|error| error.location.start())
                    .map_or(source.len(), |error| {
                        // Tokens after a complete statement or eval
                        // expression were not decoded by CPython.
                        if matches!(
                            error.error,
                            ruff_python_parser::ParseErrorType::SimpleStatementsOnSameLine
                                | ruff_python_parser::ParseErrorType::UnexpectedExpressionToken
                        ) {
                            error.location.start().to_usize()
                        } else {
                            error.location.end().to_usize()
                        }
                    });
                collect_string_escape_warnings_in_range(source, 0, end, None, &mut warnings);
            }
            if let Some(grammar) = grammar {
                grammar.status = append_relative_lazy_warnings(
                    &source_file,
                    &barry_source,
                    &parsed,
                    grammar.mode,
                    type_comments,
                    grammar.prompt,
                    &mut warnings,
                )
                .map(|(sorted, barrier)| {
                    grammar_sorted = sorted;
                    if let Some((boundary, error)) = barrier {
                        type_comment_error_offset = Some(
                            type_comment_error_offset.map_or(boundary, |old| old.min(boundary)),
                        );
                        grammar_type_comment_error = Some(error);
                    }
                    sorted || grammar_type_comment_error.is_some()
                });
            }
        } else if mode.is_none() && compiler::pre_parse_source_error(&source_file).is_ok() {
            ast_tokenizer_recovery = false;
            // Function-type input has a separate AST parser; its return
            // expression follows an arrow that expression mode rejects.
            // There are no import statements in this grammar.
            collect_string_escape_warnings_in_range(source, 0, source.len(), None, &mut warnings);
        }
        // Stable ordering retains fragment order for constants decoded by the
        // same END token. Source offsets remain separate for input boundaries.
        if !grammar_sorted {
            warnings.sort_by_key(ParserWarning::emission_offset);
        }
        // The legacy scanner cannot reconstruct separate tokenizer/parser
        // events. Token-anchored collection preserves both phases instead.
        if interpolated_body_diagnostics == InterpolatedBodyDiagnostics::Legacy
            && warnings.iter().any(|warning| {
                matches!(
                    warning,
                    ParserWarning::Escape(_, EscapeOrigin::Tokenizer, ..)
                )
            })
        {
            ast_tokenizer_recovery = false;
        }
        CollectedParserWarnings {
            warnings,
            tokenizer_recovery: ast_tokenizer_recovery,
            has_valid_syntax,
            single_boundary,
            type_comment_error_offset,
            grammar_type_comment_error,
        }
    }

    #[derive(Default)]
    pub(super) struct AstParserWarningOptions {
        pub(super) inherited_barry_as_flufl: bool,
        pub(super) type_comments: bool,
    }

    #[derive(Default)]
    struct ParserWarningBoundaries {
        single: Option<compiler::SingleModeStatementBoundary>,
        type_comment_error_offset: Option<usize>,
    }

    enum WarningEmission<'a> {
        Standard {
            emitted_escapes: &'a [usize],
            tokenizer_recovery: bool,
        },
        Ast {
            tokenizer_recovery: bool,
            original_filename: Option<&'a Py<PyStr>>,
        },
        Interactive(&'a mut super::InteractiveWarnings),
    }

    impl VirtualMachine {
        fn emit_prepared_grammar_warnings(
            &self,
            module: Option<&PyObject>,
            original_filename: Option<&Py<PyStr>>,
            prepared: PreparedGrammarWarnings,
            emitted_escapes: &[usize],
            ast_only: bool,
        ) -> Result<Vec<usize>, VmCompileError> {
            let source = prepared.source.as_ref();
            let filename = prepared.filename.as_str();
            let collected = prepared.collected;
            if collected.warnings.iter().any(|warning| {
                matches!(warning, ParserWarning::RelativeLazy { recovery: None, .. })
            }) {
                return Err(self.grammar_invariant_error(
                    filename,
                    GrammarCompositionUnavailable::MissingRecoveryPlan,
                ));
            }
            let emitted = self.emit_parser_warning_batch_with_grammar(
                source,
                filename,
                module,
                collected.warnings,
                ParserWarningBoundaries {
                    single: if !ast_only
                        || (collected.has_valid_syntax && collected.tokenizer_recovery)
                    {
                        collected.single_boundary
                    } else {
                        None
                    },
                    type_comment_error_offset: collected.type_comment_error_offset,
                },
                if ast_only {
                    WarningEmission::Ast {
                        tokenizer_recovery: collected.tokenizer_recovery,
                        original_filename,
                    }
                } else {
                    WarningEmission::Standard {
                        emitted_escapes,
                        tokenizer_recovery: collected.tokenizer_recovery,
                    }
                },
            )?;
            if let Some(error) = collected.grammar_type_comment_error {
                return Err(VmCompileError::Compile(error));
            }
            Ok(emitted)
        }

        fn emit_prepared_interactive_grammar_warnings(
            &self,
            module: Option<&PyObject>,
            prepared: PreparedPromptGrammarWarnings,
            emitted: &mut super::InteractiveWarnings,
        ) -> Result<(), VmCompileError> {
            let PreparedPromptGrammarWarnings(prepared) = prepared;
            self.emit_parser_warning_batch_with_grammar(
                &prepared.source,
                &prepared.filename,
                module,
                prepared.collected.warnings,
                ParserWarningBoundaries::default(),
                WarningEmission::Interactive(emitted),
            )
            .map(|_| ())
        }

        /// Fail closed on an internal parser/consumer construction inconsistency.
        /// This is not a diagnostic for invalid Python source. The private fresh
        /// constructors bind source, mode, original reads and recovery together;
        /// a supported-input hit is a bug, not a permitted compatibility result.
        fn grammar_invariant_error(
            &self,
            filename: &str,
            reason: GrammarCompositionUnavailable,
        ) -> VmCompileError {
            VmCompileError::Warning(CompileWarningError {
                exception: self.new_system_error(format!(
                    "internal grammar diagnostic invariant failed: {reason:?}"
                )),
                filename: filename.to_owned(),
                original_filename: None,
                lineno: 0,
                offset: 0,
                replacement: None,
                diagnostic_text: None,
                omit_source_text: true,
            })
        }

        /// A complete prepared batch, or no grammar actions at all. Failure is
        /// explicit before callbacks, never a feature-presence fallback bypass.
        pub(super) fn try_emit_standard_grammar_warnings(
            &self,
            context: WarningSource<'_>,
            module: Option<&PyObject>,
            mode: compiler::Mode,
            inherited_barry_as_flufl: bool,
            type_comments: bool,
            emitted_escapes: &[usize],
        ) -> Result<Option<Vec<usize>>, VmCompileError> {
            let WarningSource {
                source, filename, ..
            } = context;
            let prepared = prepare_relative_lazy_parser_warnings(
                source,
                filename,
                mode,
                inherited_barry_as_flufl,
                type_comments,
            )
            .map_err(|reason| self.grammar_invariant_error(filename, reason))?;
            prepared
                .map(|prepared| {
                    self.emit_prepared_grammar_warnings(
                        module,
                        None,
                        prepared,
                        emitted_escapes,
                        false,
                    )
                })
                .transpose()
        }

        pub(super) fn emit_standard_parser_warnings(
            &self,
            source: &str,
            filename: &str,
            module: Option<&PyObject>,
            mode: compiler::Mode,
            inherited_barry_as_flufl: bool,
            emitted_escapes: &[usize],
        ) -> Result<(), VmCompileError> {
            self.emit_standard_parser_warnings_with_type_comments(
                WarningSource::new(source, filename),
                module,
                mode,
                inherited_barry_as_flufl,
                false,
                emitted_escapes,
            )
        }

        pub(super) fn emit_standard_parser_warnings_with_type_comments(
            &self,
            context: WarningSource<'_>,
            module: Option<&PyObject>,
            mode: compiler::Mode,
            inherited_barry_as_flufl: bool,
            type_comments: bool,
            emitted_escapes: &[usize],
        ) -> Result<(), VmCompileError> {
            let WarningSource {
                source, filename, ..
            } = context;
            if self
                .try_emit_standard_grammar_warnings(
                    WarningSource::new(source, filename),
                    module,
                    mode,
                    inherited_barry_as_flufl,
                    type_comments,
                    emitted_escapes,
                )?
                .is_some()
            {
                return Ok(());
            }
            self.emit_legacy_standard_parser_warnings(
                source,
                filename,
                module,
                mode,
                inherited_barry_as_flufl,
                emitted_escapes,
            )
            .map_err(VmCompileError::Warning)
        }

        pub(super) fn emit_ast_parser_warnings(
            &self,
            source: &str,
            filename: &str,
            original_filename: Option<&Py<PyStr>>,
            module: Option<&Py<PyStr>>,
            start: CompileStart,
            options: AstParserWarningOptions,
        ) -> Result<Vec<usize>, VmCompileError> {
            let mode = match start {
                CompileStart::Single => Some(compiler::Mode::Single),
                CompileStart::File => Some(compiler::Mode::Exec),
                CompileStart::Eval => Some(compiler::Mode::Eval),
                CompileStart::FuncType => None,
            };
            if let Some(mode) = mode
                && let Some(prepared) = prepare_relative_lazy_parser_warnings(
                    source,
                    filename,
                    mode,
                    options.inherited_barry_as_flufl,
                    options.type_comments,
                )
                .map_err(|reason| self.grammar_invariant_error(filename, reason))?
            {
                return self.emit_prepared_grammar_warnings(
                    module.map(AsObject::as_object),
                    original_filename,
                    prepared,
                    &[],
                    true,
                );
            }
            self.emit_legacy_ast_parser_warnings(
                source,
                filename,
                original_filename,
                module,
                start,
                options,
            )
            .map_err(VmCompileError::Warning)
        }

        pub(in crate::vm) fn emit_interactive_parser_warnings(
            &self,
            source: &str,
            filename: &str,
            module: Option<&Py<PyStr>>,
            inherited_barry_as_flufl: bool,
            emitted: &mut super::InteractiveWarnings,
        ) -> Result<(), VmCompileError> {
            if let Some(prepared) = prepare_interactive_relative_lazy_warnings(
                source,
                filename,
                inherited_barry_as_flufl,
            )
            .map_err(|reason| self.grammar_invariant_error(filename, reason))?
            {
                return self.emit_prepared_interactive_grammar_warnings(
                    module.map(AsObject::as_object),
                    prepared,
                    emitted,
                );
            }
            self.emit_legacy_interactive_parser_warnings(
                source,
                filename,
                module,
                inherited_barry_as_flufl,
                emitted,
            )
            .map_err(VmCompileError::Warning)
        }

        fn emit_legacy_standard_parser_warnings(
            &self,
            source: &str,
            filename: &str,
            module: Option<&PyObject>,
            mode: compiler::Mode,
            inherited_barry_as_flufl: bool,
            emitted_escapes: &[usize],
        ) -> Result<(), CompileWarningError> {
            let implicit_final_newline = matches!(mode, compiler::Mode::Exec);
            let is_single = matches!(mode, compiler::Mode::Single);
            let mode = match mode {
                compiler::Mode::Eval => ruff_python_parser::Mode::Expression,
                compiler::Mode::Exec | compiler::Mode::Single | compiler::Mode::BlockExpr => {
                    ruff_python_parser::Mode::Module
                }
            };
            let collected = collect_parser_warnings(
                source,
                filename,
                Some(mode),
                inherited_barry_as_flufl,
                InterpolatedBodyDiagnostics::TokenAnchored {
                    implicit_final_newline,
                },
                is_single,
                false,
            );
            if !collected.has_valid_syntax {
                // Invalid source still needs the legacy recovery contract;
                // a valid mixed batch already has separate tokenizer events.
                self.emit_tokenizer_syntax_warnings(source, filename, module)?;
                return self.emit_string_escape_warnings(source, filename, module, emitted_escapes);
            }
            self.emit_parser_warning_batch(
                source,
                filename,
                module,
                collected.warnings,
                ParserWarningBoundaries {
                    single: collected.single_boundary,
                    type_comment_error_offset: None,
                },
                WarningEmission::Standard {
                    emitted_escapes,
                    tokenizer_recovery: true,
                },
            )
            .map(|_| ())
        }

        // Preserve the existing invalid-source warning order and recovery surface.
        pub(super) fn emit_tokenizer_syntax_warnings(
            &self,
            source: &str,
            filename: &str,
            module: Option<&PyObject>,
        ) -> Result<(), CompileWarningError> {
            let mut warnings = Vec::new();
            collect_numeric_literal_warnings(source, &mut warnings);
            self.emit_parser_warning_batch(
                source,
                filename,
                module,
                warnings,
                ParserWarningBoundaries::default(),
                WarningEmission::Standard {
                    emitted_escapes: &[],
                    tokenizer_recovery: false,
                },
            )
            .map(|_| ())
        }

        pub(super) fn emit_string_escape_warnings(
            &self,
            source: &str,
            filename: &str,
            module: Option<&PyObject>,
            emitted_escapes: &[usize],
        ) -> Result<(), CompileWarningError> {
            use ruff_text_size::Ranged;

            let source_file = compiler::core::SourceFileBuilder::new(filename, source).finish();
            if compiler::pre_parse_source_error(&source_file).is_err() {
                return Ok(());
            }
            let mut warnings = Vec::new();
            let parsed = ruff_python_parser::parse_unchecked(
                source,
                ruff_python_parser::Mode::Module.into(),
            );
            if parsed.has_valid_syntax() {
                let mut visitor = EscapeWarningVisitor {
                    source,
                    warnings: &mut warnings,
                    ast_only: false,
                    interpolated_body_diagnostics: InterpolatedBodyDiagnostics::Legacy,
                    diagnostic_index: core::cell::OnceCell::new(),
                    interpolation_depth: 0,
                    depth: 0,
                    depth_limit: compiler::CompileOpts::default().recursion_limit,
                };
                match parsed.syntax() {
                    ast::Mod::Module(module) => {
                        for stmt in &module.body {
                            visitor.visit_stmt(stmt);
                        }
                    }
                    ast::Mod::Expression(expr) => visitor.visit_expr(&expr.body),
                }
            } else {
                // A completed statement/expression rejected the next token
                // before CPython's string-decoder action could consume it.
                // Match the AST collector's decoder eligibility for this
                // first-pass stop, without discarding tokenizer recovery events.
                let decoder_end = parsed
                    .diagnostic_parser_progress()
                    .and_then(|progress| progress.first_failure.as_ref())
                    .filter(|failure| {
                        failure.point.original_lexical_prefix
                            && matches!(
                                failure.kind,
                                ruff_python_parser::DiagnosticParserFailureKind::Error(
                                    ruff_python_parser::ParseErrorType::SimpleStatementsOnSameLine
                                        | ruff_python_parser::ParseErrorType::UnexpectedExpressionToken
                                )
                            )
                    })
                    .map_or(source.len(), |failure| {
                        failure.point.token.range().start().to_usize()
                    });
                collect_string_escape_warnings_in_range(
                    source,
                    0,
                    source.len(),
                    None,
                    &mut warnings,
                );
                warnings.retain(|warning| {
                    !matches!(
                        warning,
                        ParserWarning::Escape(escape, EscapeOrigin::Parser, ..)
                            if escape.offset() >= decoder_end
                    )
                });
            }
            self.emit_parser_warning_batch(
                source,
                filename,
                module,
                warnings,
                ParserWarningBoundaries::default(),
                WarningEmission::Standard {
                    emitted_escapes,
                    tokenizer_recovery: false,
                },
            )
            .map(|_| ())
        }

        fn emit_legacy_ast_parser_warnings(
            &self,
            source: &str,
            filename: &str,
            original_filename: Option<&Py<PyStr>>,
            module: Option<&Py<PyStr>>,
            start: CompileStart,
            options: AstParserWarningOptions,
        ) -> Result<Vec<usize>, CompileWarningError> {
            let mode = match start {
                CompileStart::Single | CompileStart::File => Some(ruff_python_parser::Mode::Module),
                CompileStart::Eval => Some(ruff_python_parser::Mode::Expression),
                CompileStart::FuncType => None,
            };
            let collected = collect_parser_warnings(
                source,
                filename,
                mode,
                options.inherited_barry_as_flufl,
                InterpolatedBodyDiagnostics::TokenAnchored {
                    implicit_final_newline: matches!(start, CompileStart::File),
                },
                matches!(start, CompileStart::Single),
                options.type_comments,
            );
            self.emit_parser_warning_batch(
                source,
                filename,
                module.map(AsObject::as_object),
                collected.warnings,
                ParserWarningBoundaries {
                    single: if matches!(start, CompileStart::Single)
                        && collected.has_valid_syntax
                        && collected.tokenizer_recovery
                    {
                        collected.single_boundary
                    } else {
                        None
                    },
                    type_comment_error_offset: collected.type_comment_error_offset,
                },
                WarningEmission::Ast {
                    // Unlike an interactive prompt, Single compile input recovers
                    // after a parser warning becomes an error. Later numeric
                    // tokenizer warnings raised as errors still take precedence,
                    // including on a following logical line.
                    tokenizer_recovery: collected.tokenizer_recovery
                        && matches!(
                            start,
                            CompileStart::File | CompileStart::Eval | CompileStart::Single
                        ),
                    original_filename,
                },
            )
        }

        /// Emit the tokenizer warnings for the interactive input read so far.
        fn emit_legacy_interactive_parser_warnings(
            &self,
            source: &str,
            filename: &str,
            module: Option<&Py<PyStr>>,
            inherited_barry_as_flufl: bool,
            emitted: &mut super::InteractiveWarnings,
        ) -> Result<(), CompileWarningError> {
            let collected = collect_parser_warnings(
                source,
                filename,
                Some(ruff_python_parser::Mode::Module),
                inherited_barry_as_flufl,
                InterpolatedBodyDiagnostics::Legacy,
                false,
                false,
            );
            self.emit_parser_warning_batch(
                source,
                filename,
                module.map(AsObject::as_object),
                collected.warnings,
                ParserWarningBoundaries::default(),
                WarningEmission::Interactive(emitted),
            )
            .map(|_| ())
        }

        fn emit_parser_warning_batch(
            &self,
            source: &str,
            filename: &str,
            module: Option<&PyObject>,
            warnings: Vec<ParserWarning>,
            boundaries: ParserWarningBoundaries,
            emission: WarningEmission<'_>,
        ) -> Result<Vec<usize>, CompileWarningError> {
            // All existing callers use the disabled grammar collector. Only
            // emit_prepared_grammar_warnings can receive RelativeLazy variants.
            self.emit_parser_warning_batch_with_grammar(
                source, filename, module, warnings, boundaries, emission,
            )
            .map_err(|error| match error {
                VmCompileError::Warning(error) => error,
                VmCompileError::Compile(_) => unreachable!("legacy batch has no grammar recovery"),
            })
        }

        fn emit_parser_warning_batch_with_grammar(
            &self,
            source: &str,
            filename: &str,
            module: Option<&PyObject>,
            warnings: Vec<ParserWarning>,
            boundaries: ParserWarningBoundaries,
            emission: WarningEmission<'_>,
        ) -> Result<Vec<usize>, VmCompileError> {
            let ParserWarningBoundaries {
                single: single_boundary,
                type_comment_error_offset,
            } = boundaries;
            let (emitted_escapes, ast_tokenizer_recovery, original_filename, mut interactive) =
                match emission {
                    WarningEmission::Standard {
                        emitted_escapes,
                        tokenizer_recovery,
                    } => (emitted_escapes, tokenizer_recovery, None, None),
                    WarningEmission::Ast {
                        tokenizer_recovery,
                        original_filename,
                    } => (&[][..], tokenizer_recovery, original_filename, None),
                    WarningEmission::Interactive(emitted) => (&[][..], false, None, Some(emitted)),
                };
            let mut considered_escapes = alloc::collections::BTreeSet::new();
            let mut parser_error = None;
            for warning in warnings {
                let pending_error_interpolated_tokenizer = (type_comment_error_offset
                    .is_some_and(|offset| warning.offset() >= offset)
                    || matches!(
                        single_boundary,
                        Some(compiler::SingleModeStatementBoundary::Compound { end })
                            if warning.offset() >= end.to_usize()
                    ))
                    && matches!(
                        &warning,
                        ParserWarning::Escape(_, EscapeOrigin::Tokenizer, _, Some(_))
                            | ParserWarning::Numeric {
                                in_interpolation: true,
                                ..
                            }
                    );
                // A successful single input does not parse later statements.
                // Keep those numeric events for recovery after an earlier
                // parser warning escalates, rather than truncating the source.
                if parser_error.is_none()
                    && type_comment_error_offset.is_none()
                    && let Some(boundary) = single_boundary
                {
                    let end = match (boundary, &warning) {
                        (compiler::SingleModeStatementBoundary::Simple { end, .. }, _) => Some(end),
                        (
                            compiler::SingleModeStatementBoundary::Compound { end },
                            ParserWarning::Escape(_, EscapeOrigin::Parser, ..),
                        ) => Some(end),
                        // Compound input requests another token and can emit
                        // tokenizer warnings while rejecting the next statement.
                        _ => None,
                    };
                    if end.is_some_and(|end| warning.offset() >= end.to_usize()) {
                        continue;
                    }
                }
                // CPython resumes tokenization after a parser-origin SyntaxError.
                // Preserve ordinary warning order and never replay prior warnings.
                if (parser_error.is_some()
                    || type_comment_error_offset
                        .is_some_and(|offset| warning.emission_offset() >= offset))
                    && matches!(warning, ParserWarning::Escape(_, EscapeOrigin::Parser, ..))
                {
                    continue;
                }
                // Keep #9314's existing pending-error continuation. A prior
                // decoder warning suppresses every later grammar action; it
                // does not use the relative-lazy filled-frontier recovery plan.
                if parser_error.is_some() && matches!(warning, ParserWarning::RelativeLazy { .. }) {
                    continue;
                }
                if let Some(state) = interactive.as_deref_mut() {
                    let message = match &warning {
                        ParserWarning::Escape(escape, ..) => escape.warning_message(),
                        ParserWarning::Numeric { message, .. } => message.clone(),
                        ParserWarning::RelativeLazy { action, .. } => relative_lazy_message(action),
                    };
                    if !state.emitted.insert((warning.offset(), message)) {
                        continue;
                    }
                }
                match warning {
                    ParserWarning::RelativeLazy {
                        action,
                        recovery,
                        index,
                        mode,
                    } => {
                        if let Err(error) = self.emit_relative_lazy_warning(
                            source,
                            filename,
                            original_filename,
                            module,
                            &action,
                            &index,
                        ) {
                            let syntax_warning = error
                                .exception
                                .fast_isinstance(self.ctx.exceptions.syntax_warning);
                            let pending = VmCompileError::Warning(error);
                            return Err(if syntax_warning && interactive.is_none() {
                                let Some(recovery) = recovery else {
                                    return Err(self.grammar_invariant_error(
                                        filename,
                                        GrammarCompositionUnavailable::MissingRecoveryPlan,
                                    ));
                                };
                                self.emit_grammar_recovery(
                                    WarningSource {
                                        source,
                                        filename,
                                        original_filename,
                                    },
                                    module,
                                    mode,
                                    recovery,
                                    pending,
                                    &index,
                                )
                            } else {
                                // A real prompt stops on every callback error.
                                pending
                            });
                        }
                    }
                    ParserWarning::Escape(escape, origin, error_column, diagnostic) => {
                        let offset = escape.offset();
                        if !considered_escapes.insert(offset) {
                            continue;
                        }
                        if emitted_escapes.binary_search(&offset).is_err() {
                            let result = warn_invalid_escape_sequence_at(
                                source,
                                escape,
                                filename,
                                original_filename,
                                module,
                                diagnostic,
                                self,
                            )
                            .map_err(|mut error| {
                                if let Some(column) = error_column {
                                    error.offset = column;
                                    if let Some(replacement) = &mut error.replacement {
                                        replacement.end_offset = column + 2;
                                    }
                                }
                                error
                            });
                            if let Err(error) = result {
                                if ast_tokenizer_recovery
                                    && origin == EscapeOrigin::Parser
                                    && error
                                        .exception
                                        .fast_isinstance(self.ctx.exceptions.syntax_warning)
                                {
                                    parser_error = Some(error);
                                } else if origin == EscapeOrigin::Tokenizer
                                    && let Some(parser_error) = parser_error.take()
                                {
                                    // _PyPegen_tokenize_full_source preserves
                                    // the original parser error if recovery
                                    // fails while inside an interpolated string.
                                    return Err(VmCompileError::Warning(parser_error));
                                } else if pending_error_interpolated_tokenizer {
                                    // Single or TYPE_COMMENT validation already
                                    // failed. Recovery invokes the warning, but
                                    // an error inside that string must preserve
                                    // the pending syntax error. Stop here and
                                    // let the following validation report it.
                                    break;
                                } else {
                                    return Err(VmCompileError::Warning(error));
                                }
                            }
                        }
                    }
                    ParserWarning::Numeric {
                        last_offset,
                        message,
                        in_interpolation,
                        ..
                    } => {
                        let result = warn_numeric_literal(
                            source,
                            filename,
                            original_filename,
                            module,
                            last_offset,
                            message,
                            self,
                        );
                        if let Err(error) = result {
                            if in_interpolation && let Some(parser_error) = parser_error.take() {
                                return Err(VmCompileError::Warning(parser_error));
                            }
                            if pending_error_interpolated_tokenizer {
                                break;
                            }
                            return Err(VmCompileError::Warning(error));
                        }
                    }
                }
            }
            if let Some(error) = parser_error {
                return Err(VmCompileError::Warning(error));
            }
            // AST warning callbacks use binary search on these source offsets,
            // which need not be emitted in source order.
            Ok(considered_escapes.into_iter().collect())
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
                    .compile(source, compiler::Mode::Exec, "<test>")
                    .expect_err("expected compiler SyntaxWarning");
                let exception = err.into_pyexception(vm, Some(source));
                exception
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
        fn ast_only_compile_honors_barry_as_flufl() {
            Interpreter::without_stdlib(Default::default()).enter(|vm| {
                let flags = CompilerFlags::ONLY_AST.bits();
                vm.compile_string_object_with_flags(
                    b"from __future__ import barry_as_FLUFL\n2 <> 3\n",
                    "<test>",
                    CompileStart::File.as_i32(),
                    flags,
                    -1,
                    -1,
                )
                .expect("PyCF_ONLY_AST should accept <> in Barry mode");

                let err = vm
                    .compile_string_object_with_flags(
                        b"from __future__ import barry_as_FLUFL\n2 != 3\n",
                        "<test>",
                        CompileStart::File.as_i32(),
                        flags,
                        -1,
                        -1,
                    )
                    .expect_err("PyCF_ONLY_AST should reject != in Barry mode");
                assert!(
                    err.as_object()
                        .str(vm)
                        .unwrap()
                        .as_wtf8()
                        .to_string()
                        .contains("with Barry as BDFL")
                );
            });
        }

        #[test]
        fn type_comment_preparse_honors_inherited_barry_as_flufl() {
            Interpreter::without_stdlib(Default::default()).enter(|vm| {
                let flags = CompilerFlags::TYPE_COMMENTS.bits()
                    | crate::bytecode::CodeFlags::FUTURE_BARRY_AS_BDFL.bits() as i32;
                vm.compile_string_object_with_flags(
                    b"2 <> 3\n",
                    "<test>",
                    CompileStart::File.as_i32(),
                    flags,
                    -1,
                    -1,
                )
                .expect("type-comment preparse should accept <> in inherited Barry mode");
            });
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
        fn string_escape_warning_precedes_later_parse_error() {
            let message = compile_error_message("'\\e' $\n");
            assert!(
                message.contains("\"\\e\" is an invalid escape sequence"),
                "expected invalid escape before parse error, got {message:?}"
            );
        }

        #[test]
        fn invalid_octal_escape_warning_escalates() {
            let message = compile_error_message("'''\n\\407'''\n");
            assert!(
                message.contains("\"\\407\" is an invalid octal escape sequence"),
                "expected invalid octal escape, got {message:?}"
            );
        }

        #[test]
        fn unparsed_identifier_suffix_is_not_a_raw_prefix() {
            let message = compile_error_message("bar'\\z' $\n");
            assert!(
                message.contains("invalid syntax"),
                "expected syntax error before decoding the adjacent string, got {message:?}"
            );
            assert!(
                !message.contains("invalid escape"),
                "an unparsed adjacent string must not emit an escape warning, got {message:?}"
            );
        }

        #[test]
        fn unparsed_raw_prefix_after_dot_stays_raw() {
            let message = compile_error_message("obj.r'\\z'\n");
            assert!(
                !message.contains("invalid escape"),
                "r after '.' is a raw prefix, got {message:?}"
            );
        }

        #[test]
        fn unparsed_incompatible_prefix_skips_escape_scan() {
            let message = compile_error_message("ur'\\z'\n");
            assert!(
                !message.contains("invalid escape"),
                "incompatible prefixes should not emit an escape warning, got {message:?}"
            );
        }

        #[test]
        fn unparsed_raw_interpolation_does_not_warn() {
            let message = compile_error_message("f\"{r'\\z'}\" $\n");
            assert!(
                !message.contains("invalid escape"),
                "raw interpolation must not be scanned as f-string text, got {message:?}"
            );
        }

        #[test]
        fn unparsed_fstring_literal_parts_still_warn() {
            let message = compile_error_message("f\"pre\\z{r'\\e'}post\\q\" $\n");
            assert!(
                message.contains("\"\\z\" is an invalid escape sequence"),
                "f-string literal parts should still warn, got {message:?}"
            );
        }

        #[test]
        fn unparsed_nested_fstring_in_interpolation_warns() {
            let message = compile_error_message("f\"{f'\\z'}\" $\n");
            assert!(
                message.contains("\"\\z\" is an invalid escape sequence"),
                "non-raw nested f-string should warn, got {message:?}"
            );
        }

        #[test]
        fn unparsed_format_spec_escape_warns() {
            let message = compile_error_message("f\"{x:\\z}\" $\n");
            assert!(
                message.contains("\"\\z\" is an invalid escape sequence"),
                "format spec is f-string text, got {message:?}"
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
