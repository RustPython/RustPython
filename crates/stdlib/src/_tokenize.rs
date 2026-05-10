pub(crate) use _tokenize::module_def;

#[pymodule]
mod _tokenize {
    use crate::{
        common::lock::PyRwLock,
        vm::{
            AsObject, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
            builtins::{PyBytes, PyStr, PyType},
            convert::ToPyObject,
            function::ArgCallable,
            protocol::PyIterReturn,
            types::{Constructor, IterNext, Iterable, SelfIter},
        },
    };
    use core::fmt;
    use ruff_python_ast::PySourceType;
    use ruff_python_ast::token::{Token, TokenKind};
    use ruff_python_parser::{
        LexicalErrorType, ParseError, ParseErrorType, parse_unchecked_source,
    };
    use ruff_source_file::{LineIndex, LineRanges};
    use ruff_text_size::{Ranged, TextSize};

    const TOKEN_ENDMARKER: u8 = 0;
    const TOKEN_DEDENT: u8 = 6;
    const TOKEN_OP: u8 = 55;
    const TOKEN_COMMENT: u8 = 65;
    const TOKEN_NL: u8 = 66;

    #[pyattr]
    #[pyclass(name = "TokenizerIter")]
    #[derive(PyPayload)]
    pub(super) struct PyTokenizerIter {
        readline: ArgCallable,
        extra_tokens: bool,
        encoding: Option<String>,
        state: PyRwLock<TokenizerState>,
    }

    impl PyTokenizerIter {
        fn readline(&self, vm: &VirtualMachine) -> PyResult<String> {
            let raw_line = match self.readline.invoke((), vm) {
                Ok(v) => v,
                Err(err) => {
                    if err.fast_isinstance(vm.ctx.exceptions.stop_iteration) {
                        return Ok(String::new());
                    }
                    return Err(err);
                }
            };
            Ok(match &self.encoding {
                Some(encoding) => {
                    let bytes = raw_line
                        .downcast::<PyBytes>()
                        .map_err(|_| vm.new_type_error("readline() returned a non-bytes object"))?;
                    vm.state
                        .codec_registry
                        .decode_text(bytes.into(), encoding, None, vm)
                        .map(|s| s.to_string())?
                }
                None => raw_line
                    .downcast::<PyStr>()
                    .map(|s| s.to_string())
                    .map_err(|_| vm.new_type_error("readline() returned a non-string object"))?,
            })
        }
    }

    impl fmt::Debug for PyTokenizerIter {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("PyTokenizerIter")
                .field("extra_tokens", &self.extra_tokens)
                .field("encoding", &self.encoding)
                .finish()
        }
    }

    #[pyclass(with(Constructor, Iterable, IterNext))]
    impl PyTokenizerIter {}

    impl Constructor for PyTokenizerIter {
        type Args = PyTokenizerIterArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            let Self::Args {
                readline,
                extra_tokens,
                encoding,
            } = args;

            Ok(Self {
                readline,
                extra_tokens,
                encoding: encoding.map(|s| s.to_string()),
                state: PyRwLock::new(TokenizerState {
                    phase: TokenizerPhase::Reading {
                        source: String::new(),
                    },
                }),
            })
        }
    }

    impl SelfIter for PyTokenizerIter {}

    impl IterNext for PyTokenizerIter {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let mut state = zelf.state.read().clone();

            loop {
                match &mut state.phase {
                    TokenizerPhase::Reading { source } => {
                        let line = zelf.readline(vm)?;
                        if line.is_empty() {
                            let accumulated = core::mem::take(source);
                            let parsed = parse_unchecked_source(&accumulated, PySourceType::Python);
                            let tokens: Vec<Token> = parsed.tokens().iter().copied().collect();
                            let errors: Vec<ParseError> = parsed.errors().to_vec();
                            let line_index = LineIndex::from_source_text(&accumulated);
                            let implicit_nl = !accumulated.ends_with('\n');
                            state.phase = TokenizerPhase::Yielding {
                                source: accumulated,
                                tokens,
                                errors,
                                index: 0,
                                line_index,
                                need_implicit_nl: implicit_nl,
                                pending_fstring_parts: Vec::new(),
                                pending_empty_fstring_middle: None,
                            };
                        } else {
                            source.push_str(&line);
                        }
                    }
                    TokenizerPhase::Yielding { .. } => {
                        let result = emit_next_token(&mut state, zelf.extra_tokens, vm)?;
                        *zelf.state.write() = state;
                        return Ok(result);
                    }
                    TokenizerPhase::Done => {
                        return Ok(PyIterReturn::StopIteration(None));
                    }
                }
            }
        }
    }

    /// Emit the next token from the Yielding phase.
    fn emit_next_token(
        state: &mut TokenizerState,
        extra_tokens: bool,
        vm: &VirtualMachine,
    ) -> PyResult<PyIterReturn> {
        let TokenizerPhase::Yielding {
            source,
            tokens,
            errors,
            index,
            line_index,
            need_implicit_nl,
            pending_fstring_parts,
            pending_empty_fstring_middle,
        } = &mut state.phase
        else {
            unreachable!()
        };

        // Emit pending empty FSTRING_MIDDLE (for format spec nesting)
        if let Some((mid_type, mid_line, mid_col, mid_line_str)) =
            pending_empty_fstring_middle.take()
        {
            return Ok(PyIterReturn::Return(make_token_tuple(
                vm,
                mid_type,
                "",
                mid_line,
                mid_col as isize,
                mid_line,
                mid_col as isize,
                &mid_line_str,
            )));
        }

        // Emit any pending fstring sub-tokens first
        if let Some((tok_type, tok_str, sl, sc, el, ec)) = pending_fstring_parts.pop() {
            let offset: usize = source
                .lines()
                .take(sl.saturating_sub(1))
                .map(|l| l.len() + 1)
                .sum();
            let full_line = source.full_line_str(TextSize::from(offset.min(source.len()) as u32));
            return Ok(PyIterReturn::Return(make_token_tuple(
                vm,
                tok_type,
                &tok_str,
                sl,
                sc as isize,
                el,
                ec as isize,
                full_line,
            )));
        }

        let source_len = TextSize::from(source.len() as u32);

        while *index < tokens.len() {
            let token = tokens[*index];
            *index += 1;
            let kind = token.kind();
            let range = token.range();

            // Check for lexical indentation errors.
            // Skip when source has tabs — ruff and CPython handle tab
            // indentation differently (CPython uses tabsize=8), so ruff may
            // report false IndentationErrors for valid mixed-tab code.
            if !source.contains('\t') {
                for err in errors.iter() {
                    if !matches!(
                        err.error,
                        ParseErrorType::Lexical(LexicalErrorType::IndentationError)
                    ) {
                        continue;
                    }
                    if err.location.start() <= range.start() && range.start() < err.location.end() {
                        return Err(raise_indentation_error(vm, err, source, line_index));
                    }
                }
            }

            if kind == TokenKind::EndOfFile {
                continue;
            }

            if !extra_tokens && matches!(kind, TokenKind::Comment | TokenKind::NonLogicalNewline) {
                continue;
            }

            let raw_type = token_kind_value(kind);
            let token_type = if extra_tokens && raw_type > TOKEN_DEDENT && raw_type < TOKEN_OP {
                TOKEN_OP
            } else {
                raw_type
            };

            let (token_str, start_line, start_col, end_line, end_col, line_str) =
                if kind == TokenKind::Dedent {
                    let last_line = source.lines().count();
                    let default_pos = if extra_tokens {
                        (last_line + 1, 0)
                    } else {
                        (last_line, 0)
                    };
                    let (pos, dedent_line) =
                        next_non_dedent_info(tokens, *index, source, line_index, default_pos);
                    ("", pos.0, pos.1, pos.0, pos.1, dedent_line)
                } else {
                    let start_lc = line_index.line_column(range.start(), source);
                    let start_line = start_lc.line.get();
                    let start_col = start_lc.column.to_zero_indexed();
                    let implicit_newline = range.start() >= source_len;
                    let in_source = range.end() <= source_len;

                    let (s, el, ec) = if kind == TokenKind::Newline {
                        if extra_tokens {
                            if implicit_newline {
                                ("", start_line, start_col + 1)
                            } else {
                                let s = if source[range].starts_with('\r') {
                                    "\r\n"
                                } else {
                                    "\n"
                                };
                                (s, start_line, start_col + s.len())
                            }
                        } else {
                            ("", start_line, start_col)
                        }
                    } else if kind == TokenKind::NonLogicalNewline {
                        let s = if in_source { &source[range] } else { "" };
                        (s, start_line, start_col + s.len())
                    } else {
                        let end_lc = line_index.line_column(range.end(), source);
                        let s = if in_source { &source[range] } else { "" };
                        (s, end_lc.line.get(), end_lc.column.to_zero_indexed())
                    };
                    let line_str = source.full_line_str(range.start());
                    (s, start_line, start_col, el, ec, line_str)
                };

            // Handle FSTRING_MIDDLE/TSTRING_MIDDLE brace unescaping
            if matches!(kind, TokenKind::FStringMiddle | TokenKind::TStringMiddle)
                && (token_str.contains("{{") || token_str.contains("}}"))
            {
                let mut parts =
                    split_fstring_middle(token_str, token_type, start_line, start_col).into_iter();
                let (tt, ts, sl, sc, el, ec) = parts.next().unwrap();
                let rest: Vec<_> = parts.collect();
                for p in rest.into_iter().rev() {
                    pending_fstring_parts.push(p);
                }
                return Ok(PyIterReturn::Return(make_token_tuple(
                    vm,
                    tt,
                    &ts,
                    sl,
                    sc as isize,
                    el,
                    ec as isize,
                    line_str,
                )));
            }

            // After emitting a Rbrace inside an fstring, check if the
            // next token is also Rbrace without an intervening FStringMiddle.
            // CPython emits an empty FSTRING_MIDDLE in that position.
            if kind == TokenKind::Rbrace
                && tokens
                    .get(*index)
                    .is_some_and(|t| t.kind() == TokenKind::Rbrace)
            {
                let mid_type = find_fstring_middle_type(tokens, *index);
                *pending_empty_fstring_middle =
                    Some((mid_type, end_line, end_col, line_str.to_string()));
            }

            return Ok(PyIterReturn::Return(make_token_tuple(
                vm,
                token_type,
                token_str,
                start_line,
                start_col as isize,
                end_line,
                end_col as isize,
                line_str,
            )));
        }

        // Emit implicit NL before ENDMARKER if source
        // doesn't end with newline and last token is Comment
        if extra_tokens && core::mem::take(need_implicit_nl) {
            let last_tok = tokens
                .iter()
                .rev()
                .find(|t| t.kind() != TokenKind::EndOfFile);
            if let Some(last) = last_tok.filter(|t| t.kind() == TokenKind::Comment) {
                let end_lc = line_index.line_column(last.range().end(), source);
                let nl_line = end_lc.line.get();
                let nl_col = end_lc.column.to_zero_indexed();
                return Ok(PyIterReturn::Return(make_token_tuple(
                    vm,
                    TOKEN_NL,
                    "",
                    nl_line,
                    nl_col as isize,
                    nl_line,
                    nl_col as isize + 1,
                    source.full_line_str(last.range().start()),
                )));
            }
        }

        // Check for unclosed brackets before ENDMARKER — CPython's tokenizer
        // raises SyntaxError("EOF in multi-line statement") in this case.
        {
            let bracket_count: i32 = tokens
                .iter()
                .map(|t| match t.kind() {
                    TokenKind::Lpar | TokenKind::Lsqb | TokenKind::Lbrace => 1,
                    TokenKind::Rpar | TokenKind::Rsqb | TokenKind::Rbrace => -1,
                    _ => 0,
                })
                .sum();
            if bracket_count > 0 {
                let last_line = source.lines().count();
                return Err(raise_syntax_error(
                    vm,
                    "EOF in multi-line statement",
                    last_line + 1,
                    0,
                ));
            }
        }

        // All tokens consumed — emit ENDMARKER
        let last_line = source.lines().count();
        let (em_line, em_col, em_line_str): (usize, isize, &str) = if extra_tokens {
            (last_line + 1, 0, "")
        } else {
            let last_line_text =
                source.full_line_str(TextSize::from(source.len().saturating_sub(1) as u32));
            (last_line, -1, last_line_text)
        };

        let result = make_token_tuple(
            vm,
            TOKEN_ENDMARKER,
            "",
            em_line,
            em_col,
            em_line,
            em_col,
            em_line_str,
        );
        state.phase = TokenizerPhase::Done;
        Ok(PyIterReturn::Return(result))
    }

    /// Determine whether to emit FSTRING_MIDDLE (60) or TSTRING_MIDDLE (63)
    /// by looking back for the most recent FStringStart/TStringStart.
    fn find_fstring_middle_type(tokens: &[Token], index: usize) -> u8 {
        let mut depth = 0i32;
        for i in (0..index).rev() {
            match tokens[i].kind() {
                TokenKind::FStringEnd | TokenKind::TStringEnd => depth += 1,
                TokenKind::FStringStart => {
                    if depth == 0 {
                        return 60; // FSTRING_MIDDLE
                    }
                    depth -= 1;
                }
                TokenKind::TStringStart => {
                    if depth == 0 {
                        return 63; // TSTRING_MIDDLE
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        60 // default to FSTRING_MIDDLE
    }

    /// Find the next non-DEDENT token's position and source line.
    /// Returns ((line, col), line_str).
    fn next_non_dedent_info<'a>(
        tokens: &[Token],
        index: usize,
        source: &'a str,
        line_index: &LineIndex,
        default_pos: (usize, usize),
    ) -> ((usize, usize), &'a str) {
        for future in &tokens[index..] {
            match future.kind() {
                TokenKind::Dedent => continue,
                TokenKind::EndOfFile => return (default_pos, ""),
                _ => {
                    let flc = line_index.line_column(future.range().start(), source);
                    let pos = (flc.line.get(), flc.column.to_zero_indexed());
                    return (pos, source.full_line_str(future.range().start()));
                }
            }
        }
        (default_pos, "")
    }

    /// Raise a SyntaxError with the given message and position.
    fn raise_syntax_error(
        vm: &VirtualMachine,
        msg: &str,
        lineno: usize,
        offset: usize,
    ) -> rustpython_vm::builtins::PyBaseExceptionRef {
        let exc = vm.new_exception_msg(vm.ctx.exceptions.syntax_error.to_owned(), msg.into());
        let obj = exc.as_object();
        let _ = obj.set_attr("msg", vm.ctx.new_str(msg), vm);
        let _ = obj.set_attr("lineno", vm.ctx.new_int(lineno), vm);
        let _ = obj.set_attr("offset", vm.ctx.new_int(offset), vm);
        let _ = obj.set_attr("filename", vm.ctx.new_str("<string>"), vm);
        let _ = obj.set_attr("text", vm.ctx.none(), vm);
        exc
    }

    /// Raise an IndentationError from a parse error.
    fn raise_indentation_error(
        vm: &VirtualMachine,
        err: &ParseError,
        source: &str,
        line_index: &LineIndex,
    ) -> rustpython_vm::builtins::PyBaseExceptionRef {
        let err_lc = line_index.line_column(err.location.start(), source);
        let err_line_text = source.full_line_str(err.location.start());
        let err_text = err_line_text.trim_end_matches('\n').trim_end_matches('\r');
        let msg = format!("{}", err.error);
        let exc = vm.new_exception_msg(
            vm.ctx.exceptions.indentation_error.to_owned(),
            msg.clone().into(),
        );
        let obj = exc.as_object();
        let _ = obj.set_attr("lineno", vm.ctx.new_int(err_lc.line.get()), vm);
        let _ = obj.set_attr("offset", vm.ctx.new_int(err_text.len() as i64 + 1), vm);
        let _ = obj.set_attr("msg", vm.ctx.new_str(msg), vm);
        let _ = obj.set_attr("filename", vm.ctx.new_str("<string>"), vm);
        let _ = obj.set_attr("text", vm.ctx.new_str(err_text), vm);
        exc
    }

    /// Split an FSTRING_MIDDLE/TSTRING_MIDDLE token containing `{{`/`}}`
    /// into multiple unescaped sub-tokens.
    /// Returns vec of (type, string, start_line, start_col, end_line, end_col).
    fn split_fstring_middle(
        raw: &str,
        token_type: u8,
        start_line: usize,
        start_col: usize,
    ) -> Vec<(u8, String, usize, usize, usize, usize)> {
        let mut parts = Vec::new();
        let mut current = String::new();
        // Track source position (line, col) — these correspond to the
        // original source positions (with {{ and }} still doubled)
        let mut cur_line = start_line;
        let mut cur_col = start_col;
        // Track the start position of the current accumulating part
        let mut part_start_line = cur_line;
        let mut part_start_col = cur_col;
        let mut chars = raw.chars().peekable();

        // Compute end position of the current accumulated text
        let end_pos = |current: &str, start_line: usize, start_col: usize| -> (usize, usize) {
            let mut el = start_line;
            let mut ec = start_col;
            for ch in current.chars() {
                if ch == '\n' {
                    el += 1;
                    ec = 0;
                } else {
                    ec += ch.len_utf8();
                }
            }
            (el, ec)
        };

        while let Some(ch) = chars.next() {
            if ch == '{' && chars.peek() == Some(&'{') {
                chars.next();
                current.push('{');
                cur_col += 2; // skip both {{ in source
            } else if ch == '}' && chars.peek() == Some(&'}') {
                chars.next();
                // Flush accumulated text before }}
                if !current.is_empty() {
                    let (el, ec) = end_pos(&current, part_start_line, part_start_col);
                    parts.push((
                        token_type,
                        core::mem::take(&mut current),
                        part_start_line,
                        part_start_col,
                        el,
                        ec,
                    ));
                }
                // Emit unescaped '}' at source position of }}
                parts.push((
                    token_type,
                    "}".to_string(),
                    cur_line,
                    cur_col,
                    cur_line,
                    cur_col + 1,
                ));
                cur_col += 2; // skip both }} in source
                part_start_line = cur_line;
                part_start_col = cur_col;
            } else {
                if current.is_empty() {
                    part_start_line = cur_line;
                    part_start_col = cur_col;
                }
                current.push(ch);
                if ch == '\n' {
                    cur_line += 1;
                    cur_col = 0;
                } else {
                    cur_col += ch.len_utf8();
                }
            }
        }

        if !current.is_empty() {
            let (el, ec) = end_pos(&current, part_start_line, part_start_col);
            parts.push((token_type, current, part_start_line, part_start_col, el, ec));
        }

        parts
    }

    #[allow(clippy::too_many_arguments)]
    fn make_token_tuple(
        vm: &VirtualMachine,
        token_type: u8,
        string: &str,
        start_line: usize,
        start_col: isize,
        end_line: usize,
        end_col: isize,
        line: &str,
    ) -> PyObjectRef {
        vm.ctx
            .new_tuple(vec![
                token_type.to_pyobject(vm),
                vm.ctx.new_str(string).into(),
                vm.ctx
                    .new_tuple(vec![start_line.to_pyobject(vm), start_col.to_pyobject(vm)])
                    .into(),
                vm.ctx
                    .new_tuple(vec![end_line.to_pyobject(vm), end_col.to_pyobject(vm)])
                    .into(),
                vm.ctx.new_str(line).into(),
            ])
            .into()
    }

    #[derive(FromArgs)]
    pub(super) struct PyTokenizerIterArgs {
        #[pyarg(positional)]
        readline: ArgCallable,
        #[pyarg(named)]
        extra_tokens: bool,
        #[pyarg(named, optional)]
        encoding: Option<rustpython_vm::PyRef<PyStr>>,
    }

    #[derive(Clone, Debug)]
    struct TokenizerState {
        phase: TokenizerPhase,
    }

    #[derive(Clone, Debug)]
    enum TokenizerPhase {
        Reading {
            source: String,
        },
        Yielding {
            source: String,
            tokens: Vec<Token>,
            errors: Vec<ParseError>,
            index: usize,
            line_index: LineIndex,
            need_implicit_nl: bool,
            /// Pending sub-tokens from FSTRING_MIDDLE splitting
            pending_fstring_parts: Vec<(u8, String, usize, usize, usize, usize)>,
            /// Pending empty FSTRING_MIDDLE for format spec nesting:
            /// (type, line, col, line_str)
            pending_empty_fstring_middle: Option<(u8, usize, usize, String)>,
        },
        Done,
    }

    const fn token_kind_value(kind: TokenKind) -> u8 {
        match kind {
            TokenKind::EndOfFile => 0,
            TokenKind::Name
            | TokenKind::For
            | TokenKind::In
            | TokenKind::Pass
            | TokenKind::Class
            | TokenKind::And
            | TokenKind::Is
            | TokenKind::Raise
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Assert
            | TokenKind::Try
            | TokenKind::While
            | TokenKind::Yield
            | TokenKind::Lambda
            | TokenKind::None
            | TokenKind::Not
            | TokenKind::Or
            | TokenKind::Break
            | TokenKind::Continue
            | TokenKind::Global
            | TokenKind::Nonlocal
            | TokenKind::Return
            | TokenKind::Except
            | TokenKind::Import
            | TokenKind::Case
            | TokenKind::Match
            | TokenKind::Type
            | TokenKind::Await
            | TokenKind::With
            | TokenKind::Del
            | TokenKind::Finally
            | TokenKind::From
            | TokenKind::Def
            | TokenKind::If
            | TokenKind::Else
            | TokenKind::Elif
            | TokenKind::As
            | TokenKind::Async => 1,
            TokenKind::Int | TokenKind::Complex | TokenKind::Float => 2,
            TokenKind::String => 3,
            TokenKind::Newline => 4,
            TokenKind::NonLogicalNewline => TOKEN_NL,
            TokenKind::Indent => 5,
            TokenKind::Dedent => 6,
            TokenKind::Lpar => 7,
            TokenKind::Rpar => 8,
            TokenKind::Lsqb => 9,
            TokenKind::Rsqb => 10,
            TokenKind::Colon => 11,
            TokenKind::Comma => 12,
            TokenKind::Semi => 13,
            TokenKind::Plus => 14,
            TokenKind::Minus => 15,
            TokenKind::Star => 16,
            TokenKind::Slash => 17,
            TokenKind::Vbar => 18,
            TokenKind::Amper => 19,
            TokenKind::Less => 20,
            TokenKind::Greater => 21,
            TokenKind::Equal => 22,
            TokenKind::Dot => 23,
            TokenKind::Percent => 24,
            TokenKind::Lbrace => 25,
            TokenKind::Rbrace => 26,
            TokenKind::EqEqual => 27,
            TokenKind::NotEqual => 28,
            TokenKind::LessEqual => 29,
            TokenKind::GreaterEqual => 30,
            TokenKind::Tilde => 31,
            TokenKind::CircumFlex => 32,
            TokenKind::LeftShift => 33,
            TokenKind::RightShift => 34,
            TokenKind::DoubleStar => 35,
            TokenKind::PlusEqual => 36,
            TokenKind::MinusEqual => 37,
            TokenKind::StarEqual => 38,
            TokenKind::SlashEqual => 39,
            TokenKind::PercentEqual => 40,
            TokenKind::AmperEqual => 41,
            TokenKind::VbarEqual => 42,
            TokenKind::CircumflexEqual => 43,
            TokenKind::LeftShiftEqual => 44,
            TokenKind::RightShiftEqual => 45,
            TokenKind::DoubleStarEqual => 46,
            TokenKind::DoubleSlash => 47,
            TokenKind::DoubleSlashEqual => 48,
            TokenKind::At => 49,
            TokenKind::AtEqual => 50,
            TokenKind::Rarrow => 51,
            TokenKind::Ellipsis => 52,
            TokenKind::ColonEqual => 53,
            TokenKind::Exclamation => 54,
            TokenKind::FStringStart => 59,
            TokenKind::FStringMiddle => 60,
            TokenKind::FStringEnd => 61,
            TokenKind::Comment => TOKEN_COMMENT,
            TokenKind::TStringStart => 62,
            TokenKind::TStringMiddle => 63,
            TokenKind::TStringEnd => 64,
            TokenKind::IpyEscapeCommand | TokenKind::Question | TokenKind::Unknown => 67, // ERRORTOKEN
            TokenKind::Lazy => u8::MAX, // Placeholder: RustPython Doesn't support `lazy imports` yet
        }
    }
}
