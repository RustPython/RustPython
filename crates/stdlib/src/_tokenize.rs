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
            vm::thread::PyOperation,
        },
    };
    use alloc::sync::Arc;
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
        operation: PyOperation,
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

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let Self::Args {
                readline,
                extra_tokens,
                encoding,
            } = args;

            Ok(Self {
                readline,
                extra_tokens,
                encoding: encoding.map(|s| s.to_string()),
                operation: PyOperation::new(vm),
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
            zelf.operation.with(vm, || {
                loop {
                    // No payload borrow survives readline, codec callbacks, or Python
                    // object construction. The operation can suspend or be reentered.
                    let reading = matches!(zelf.state.read().phase, TokenizerPhase::Reading { .. });
                    if reading {
                        let line = zelf.readline(vm)?;
                        let mut state = zelf.state.write();
                        // Another next() may have completed input while readline ran.
                        // Keep its frozen token snapshot and cursor; EOF is sticky.
                        // This remains an eager tokenizer, not CPython's streaming
                        // buffer/line-metadata model for reentrant callbacks.
                        if let TokenizerPhase::Reading { source } = &mut state.phase {
                            if line.is_empty() {
                                let accumulated = core::mem::take(source);
                                let parsed =
                                    parse_unchecked_source(&accumulated, PySourceType::Python);
                                let tokens = parsed.tokens().iter().copied().collect();
                                let errors = parsed.errors().to_vec();
                                let line_index = LineIndex::from_source_text(&accumulated);
                                let implicit_nl = !accumulated.ends_with('\n');
                                state.phase = TokenizerPhase::Yielding {
                                    data: Arc::new(TokenizedSource {
                                        source: accumulated,
                                        tokens,
                                        errors,
                                        line_index,
                                    }),
                                    index: 0,
                                    need_implicit_nl: implicit_nl,
                                    pending_fstring_parts: None,
                                    pending_empty_fstring_middle: None,
                                };
                            } else {
                                source.push_str(&line);
                            }
                        }
                        continue;
                    }
                    let result = {
                        let mut shared = zelf.state.write();
                        match shared.phase {
                            TokenizerPhase::Reading { .. } => continue,
                            TokenizerPhase::Done => return Ok(PyIterReturn::StopIteration(None)),
                            TokenizerPhase::Yielding { .. } => {}
                        }
                        // This native-only transaction never crosses a callback.
                        // Preserve the existing cursor on lexical errors.
                        let mut next = shared.clone();
                        let result = emit_next_token(&mut next, zelf.extra_tokens);
                        if result.is_ok() {
                            *shared = next;
                        }
                        result
                    };
                    return result
                        .map(|token| PyIterReturn::Return(token.into_object(vm)))
                        .map_err(|error| error.into_exception(vm));
                }
            })
        }
    }

    /// Emit the next token from the Yielding phase.
    fn emit_next_token(
        state: &mut TokenizerState,
        extra_tokens: bool,
    ) -> Result<TokenData, TokenError> {
        let TokenizerPhase::Yielding {
            data,
            index,
            need_implicit_nl,
            pending_fstring_parts,
            pending_empty_fstring_middle,
        } = &mut state.phase
        else {
            unreachable!()
        };
        let TokenizedSource {
            source,
            tokens,
            errors,
            line_index,
        } = data.as_ref();

        // Emit pending empty FSTRING_MIDDLE (for format spec nesting)
        if let Some((mid_type, mid_line, mid_col, mid_line_str)) =
            pending_empty_fstring_middle.take()
        {
            return Ok(TokenData::new(
                mid_type,
                "",
                mid_line,
                mid_col as isize,
                mid_line,
                mid_col as isize,
                &mid_line_str,
            ));
        }

        // Emit any pending fstring sub-tokens first
        if let Some(pending) = pending_fstring_parts {
            let (tok_type, tok_str, sl, sc, el, ec) = &pending.parts[pending.index];
            let full_line = source.full_line_str(line_index.line_starts()[sl - 1]);
            let token = TokenData::new(
                *tok_type,
                tok_str,
                *sl,
                *sc as isize,
                *el,
                *ec as isize,
                full_line,
            );
            pending.index += 1;
            if pending.index == pending.parts.len() {
                *pending_fstring_parts = None;
            }
            return Ok(token);
        }

        let source_len = TextSize::from(source.len() as u32);

        // Ruff has no NL token for a final indentation-only line without a LF.
        if extra_tokens
            && *need_implicit_nl
            && !source.is_empty()
            && errors.is_empty()
            && tokens[*index..]
                .iter()
                .all(|t| matches!(t.kind(), TokenKind::Dedent | TokenKind::EndOfFile))
        {
            let final_line = last_physical_line(source);
            if !final_line.is_empty()
                && final_line
                    .bytes()
                    .all(|b| matches!(b, b' ' | b'\t' | b'\x0c'))
            {
                *need_implicit_nl = false;
                let end_lc = line_index.line_column(source_len, source);
                let line = end_lc.line.get();
                let column = end_lc.column.to_zero_indexed() as isize;
                return Ok(TokenData::new(
                    TOKEN_NL,
                    "",
                    line,
                    column,
                    line,
                    column + 1,
                    final_line,
                ));
            }
        }

        // A comment's implicit NL precedes the terminal DEDENT sequence.
        if extra_tokens
            && *need_implicit_nl
            && tokens
                .get(*index)
                .is_some_and(|t| t.kind() == TokenKind::Dedent)
            && tokens[*index..]
                .iter()
                .all(|t| matches!(t.kind(), TokenKind::Dedent | TokenKind::EndOfFile))
            && let Some(comment) =
                last_non_dedent_token(tokens).filter(|t| t.kind() == TokenKind::Comment)
        {
            *need_implicit_nl = false;
            return Ok(implicit_comment_newline(comment, source, line_index));
        }

        while *index < tokens.len() {
            let token = tokens[*index];
            *index += 1;
            let kind = token.kind();
            let range = token.range();

            // An indentation error replaces the token the lexer raised it for.
            if kind == TokenKind::Unknown
                && let Some(err) = errors.iter().find(|err| {
                    line_index.line_index(err.location.start())
                        == line_index.line_index(range.start())
                        && matches!(
                            err.error,
                            ParseErrorType::Lexical(
                                LexicalErrorType::IndentationError
                                    | LexicalErrorType::TabError
                                    | LexicalErrorType::TooDeepIndentation
                            )
                        )
                })
            {
                return Err(TokenError::Indentation {
                    error: err.clone(),
                    data: Arc::clone(data),
                });
            }

            // Ruff emits an empty Unknown and a synthetic tail when recovering
            // from an open bracket at EOF. They are not source tokens.
            if kind == TokenKind::Unknown
                && range.is_empty()
                && range.start() == source_len
                && has_unclosed_brackets(tokens, source)
                && errors.iter().any(|err| {
                    let ParseErrorType::Lexical(LexicalErrorType::UnclosedBracket {
                        opening,
                        incomplete: true,
                    }) = &err.error
                    else {
                        return false;
                    };
                    tokens.iter().any(|opener| {
                        !opener.range().is_empty()
                            && opener.range().start() == err.location.start()
                            && matches!(
                                (*opening, opener.kind()),
                                ('(', TokenKind::Lpar)
                                    | ('[', TokenKind::Lsqb)
                                    | ('{', TokenKind::Lbrace)
                            )
                    })
                })
            {
                *index = tokens.len();
                if extra_tokens && core::mem::take(need_implicit_nl) {
                    let end = line_index.line_column(source_len, source);
                    let line = end.line.get();
                    let column = end.column.to_zero_indexed() as isize;
                    return Ok(TokenData::new(
                        TOKEN_NL,
                        "",
                        line,
                        column,
                        line,
                        column + 1,
                        last_physical_line(source),
                    ));
                }
                break;
            }

            if kind == TokenKind::EndOfFile {
                continue;
            }

            if !extra_tokens && matches!(kind, TokenKind::Comment | TokenKind::NonLogicalNewline) {
                continue;
            }

            let raw_type = token_kind_value(kind);
            let leading_zero_number = extra_tokens
                && kind == TokenKind::Unknown
                && errors.iter().any(|err| {
                    matches!(
                        err.error,
                        ParseErrorType::Lexical(LexicalErrorType::LeadingZerosInDecimalInteger)
                    ) && err.location.start() == range.start()
                        && err.location.end() > range.start()
                        && err.location.end() < range.end()
                });
            // Ruff diagnoses the zero prefix but retains the complete numeric token.
            // CPython accepts that token as NUMBER only in extra-token mode.
            let token_type = if leading_zero_number {
                token_kind_value(TokenKind::Int)
            } else if extra_tokens
                && (kind == TokenKind::Unknown || (raw_type > TOKEN_DEDENT && raw_type < TOKEN_OP))
            {
                TOKEN_OP
            } else {
                raw_type
            };

            let (token_str, start_line, start_col, end_line, end_col, line_str) =
                if !extra_tokens && kind == TokenKind::Indent {
                    // Explicit continuations can span the indentation range.
                    // CPython reports the line reached after consuming it.
                    let line = line_index.line_column(range.end(), source).line.get();
                    ("", line, 0, line, 0, source.full_line_str(range.end()))
                } else if kind == TokenKind::Dedent {
                    let last_line = source.lines().count();
                    let default_pos = if extra_tokens {
                        (last_line + 1, 0)
                    } else {
                        (last_line, 0)
                    };
                    let default_line = if extra_tokens {
                        ""
                    } else {
                        non_extra_eof_line(tokens, source, line_index)
                    };
                    let (pos, dedent_line) = next_non_dedent_info(
                        tokens,
                        *index,
                        source,
                        line_index,
                        (default_pos, default_line),
                    );
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
                        if range.end() == source_len && s == "\r" && *need_implicit_nl {
                            // CPython appends LF to a readline chunk ending in bare CR.
                            *need_implicit_nl = false;
                            ("", start_line, start_col + 2)
                        } else {
                            (s, start_line, start_col + s.len())
                        }
                    } else {
                        let end_lc = line_index.line_column(range.end(), source);
                        let s = if in_source { &source[range] } else { "" };
                        (s, end_lc.line.get(), end_lc.column.to_zero_indexed())
                    };
                    let line_str = if kind == TokenKind::String && in_source {
                        source.full_lines_str(range)
                    } else {
                        source.full_line_str(range.start())
                    };
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
                *pending_fstring_parts = (!rest.is_empty()).then(|| PendingFStringParts {
                    parts: rest.into(),
                    index: 0,
                });
                return Ok(TokenData::new(
                    tt,
                    &ts,
                    sl,
                    sc as isize,
                    el,
                    ec as isize,
                    line_str,
                ));
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

            // The non-extra tokenizer leaves INDENT/DEDENT token pointers unset.
            let (token_str, start_col, end_col) =
                if !extra_tokens && matches!(kind, TokenKind::Indent | TokenKind::Dedent) {
                    ("", -1, -1)
                } else {
                    (token_str, start_col as isize, end_col as isize)
                };

            return Ok(TokenData::new(
                token_type, token_str, start_line, start_col, end_line, end_col, line_str,
            ));
        }

        // Emit implicit NL before ENDMARKER if source
        // doesn't end with newline and last token is Comment
        if extra_tokens && core::mem::take(need_implicit_nl) {
            let last_tok = tokens
                .iter()
                .rev()
                .find(|t| t.kind() != TokenKind::EndOfFile);
            if let Some(last) = last_tok.filter(|t| t.kind() == TokenKind::Comment) {
                return Ok(implicit_comment_newline(last, source, line_index));
            }
        }

        // Check for unclosed brackets before ENDMARKER — CPython's tokenizer
        // raises SyntaxError("unexpected EOF in multi-line statement") in this case.
        {
            if has_unclosed_brackets(tokens, source) {
                let last_line = source.lines().count();
                return Err(TokenError::UnclosedBrackets { line: last_line });
            }
        }

        // All tokens consumed — emit ENDMARKER
        let last_line = source.lines().count();
        let (em_line, em_col, em_line_str): (usize, isize, &str) = if extra_tokens {
            (last_line + 1, 0, "")
        } else {
            (
                last_line,
                -1,
                non_extra_eof_line(tokens, source, line_index),
            )
        };

        let result = TokenData::new(
            TOKEN_ENDMARKER,
            "",
            em_line,
            em_col,
            em_line,
            em_col,
            em_line_str,
        );
        state.phase = TokenizerPhase::Done;
        Ok(result)
    }

    fn has_unclosed_brackets(tokens: &[Token], source: &str) -> bool {
        let bracket_count: i32 = tokens
            .iter()
            .map(|t| match t.kind() {
                TokenKind::Lpar | TokenKind::Lsqb | TokenKind::Lbrace => 1,
                TokenKind::Rpar | TokenKind::Rsqb | TokenKind::Rbrace => -1,
                // `$` inside an f-string interpolation is Unknown, and the
                // matching `}` is then also Unknown rather than Rbrace.
                TokenKind::Unknown => match &source[t.range()] {
                    "(" | "[" | "{" => 1,
                    ")" | "]" | "}" => -1,
                    _ => 0,
                },
                _ => 0,
            })
            .sum();
        bracket_count > 0
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

    fn last_physical_line(source: &str) -> &str {
        source.char_indices().next_back().map_or("", |(offset, _)| {
            source.full_line_str(TextSize::from(offset as u32))
        })
    }

    fn last_non_dedent_token(tokens: &[Token]) -> Option<&Token> {
        tokens
            .iter()
            .rev()
            .find(|t| !matches!(t.kind(), TokenKind::Dedent | TokenKind::EndOfFile))
    }

    fn non_extra_eof_line<'a>(
        tokens: &[Token],
        source: &'a str,
        line_index: &LineIndex,
    ) -> &'a str {
        let Some(last) = last_non_dedent_token(tokens) else {
            return "";
        };
        if matches!(
            last.kind(),
            TokenKind::Comment | TokenKind::NonLogicalNewline
        ) || line_index
            .line_column(last.range().start(), source)
            .line
            .get()
            != source.lines().count()
        {
            // Ignored trailing lines never populated the non-extra line cache.
            ""
        } else {
            last_physical_line(source)
        }
    }

    fn implicit_comment_newline(
        comment: &Token,
        source: &str,
        line_index: &LineIndex,
    ) -> TokenData {
        let end_lc = line_index.line_column(comment.range().end(), source);
        let line = end_lc.line.get();
        let column = end_lc.column.to_zero_indexed() as isize;
        TokenData::new(
            TOKEN_NL,
            "",
            line,
            column,
            line,
            column + 1,
            source.full_line_str(comment.range().start()),
        )
    }

    /// Find the next non-DEDENT token's position and source line.
    /// Returns ((line, col), line_str).
    fn next_non_dedent_info<'a>(
        tokens: &[Token],
        index: usize,
        source: &'a str,
        line_index: &LineIndex,
        default_info: ((usize, usize), &'a str),
    ) -> ((usize, usize), &'a str) {
        for future in &tokens[index..] {
            match future.kind() {
                TokenKind::Dedent => continue,
                TokenKind::EndOfFile => return default_info,
                _ => {
                    let flc = line_index.line_column(future.range().start(), source);
                    let pos = (flc.line.get(), flc.column.to_zero_indexed());
                    return (pos, source.full_line_str(future.range().start()));
                }
            }
        }
        default_info
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

    /// Raise an IndentationError or TabError from a parse error.
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
        let exc_type = if matches!(
            err.error,
            ParseErrorType::Lexical(LexicalErrorType::TabError)
        ) {
            vm.ctx.exceptions.tab_error
        } else {
            vm.ctx.exceptions.indentation_error
        };
        let exc = vm.new_exception_msg(exc_type.to_owned(), msg.clone().into());
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
    ) -> Vec<FStringPart> {
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

    struct TokenData {
        token_type: u8,
        string: String,
        start: (usize, isize),
        end: (usize, isize),
        line: String,
    }

    impl TokenData {
        #[allow(clippy::too_many_arguments)]
        fn new(
            token_type: u8,
            string: &str,
            start_line: usize,
            start_col: isize,
            end_line: usize,
            end_col: isize,
            line: &str,
        ) -> Self {
            Self {
                token_type,
                string: string.to_owned(),
                start: (start_line, start_col),
                end: (end_line, end_col),
                line: line.to_owned(),
            }
        }

        fn into_object(self, vm: &VirtualMachine) -> PyObjectRef {
            vm.ctx
                .new_tuple(vec![
                    self.token_type.to_pyobject(vm),
                    vm.ctx.new_str(self.string).into(),
                    vm.ctx
                        .new_tuple(vec![
                            self.start.0.to_pyobject(vm),
                            self.start.1.to_pyobject(vm),
                        ])
                        .into(),
                    vm.ctx
                        .new_tuple(vec![self.end.0.to_pyobject(vm), self.end.1.to_pyobject(vm)])
                        .into(),
                    vm.ctx.new_str(self.line).into(),
                ])
                .into()
        }
    }

    enum TokenError {
        Indentation {
            error: ParseError,
            data: Arc<TokenizedSource>,
        },
        UnclosedBrackets {
            line: usize,
        },
    }

    impl TokenError {
        fn into_exception(
            self,
            vm: &VirtualMachine,
        ) -> rustpython_vm::builtins::PyBaseExceptionRef {
            match self {
                Self::Indentation { error, data } => {
                    raise_indentation_error(vm, &error, &data.source, &data.line_index)
                }
                Self::UnclosedBrackets { line } => {
                    let exc =
                        raise_syntax_error(vm, "unexpected EOF in multi-line statement", line, 0);
                    // PyErr_SyntaxLocationObject sets the end line to the current line.
                    if let Err(error) =
                        exc.as_object()
                            .set_attr("end_lineno", vm.ctx.new_int(line), vm)
                    {
                        return error;
                    }
                    exc
                }
            }
        }
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

    type FStringPart = (u8, String, usize, usize, usize, usize);

    #[derive(Clone, Debug)]
    struct PendingFStringParts {
        parts: Arc<[FStringPart]>,
        index: usize,
    }

    #[derive(Debug)]
    struct TokenizedSource {
        source: String,
        tokens: Vec<Token>,
        errors: Vec<ParseError>,
        line_index: LineIndex,
    }

    #[derive(Clone, Debug)]
    enum TokenizerPhase {
        Reading {
            source: String,
        },
        Yielding {
            data: Arc<TokenizedSource>,
            index: usize,
            need_implicit_nl: bool,
            /// Pending sub-tokens from FSTRING_MIDDLE splitting
            pending_fstring_parts: Option<PendingFStringParts>,
            /// Pending empty FSTRING_MIDDLE for format spec nesting:
            /// (type, line, col, line_str)
            pending_empty_fstring_middle: Option<(u8, usize, usize, String)>,
        },
        Done,
    }

    const fn token_kind_value(kind: TokenKind) -> u8 {
        match kind {
            TokenKind::EndOfFile => 0,
            TokenKind::Identifier
            | TokenKind::Lazy
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
        }
    }
}
