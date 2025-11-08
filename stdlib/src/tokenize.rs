pub(crate) use _tokenize::make_module;

#[pymodule]
mod _tokenize {
    use crate::{
        common::lock::PyRwLock,
        vm::{
            Py, PyPayload, PyResult, VirtualMachine,
            builtins::{PyBytes, PyStr, PyStrRef, PyTypeRef},
            convert::ToPyObject,
            function::ArgCallable,
            protocol::PyIterReturn,
            types::{Constructor, IterNext, Iterable, SelfIter},
        },
    };
    use ruff_python_ast::PySourceType;
    use ruff_python_parser::{ParseError, Token, TokenKind, Tokens, parse_unchecked_source};
    use ruff_source_file::{LineIndex, LineRanges};
    use ruff_text_size::{Ranged, TextRange};
    use std::{cmp::Ordering, fmt};

    /// Cpython `__import__("token").OP`
    const TOKEN_OP: u8 = 55;

    #[pyattr]
    #[pyclass(name = "TokenizerIter")]
    #[derive(PyPayload)]
    pub struct PyTokenizerIter {
        readline: ArgCallable, // TODO: This should be PyObject
        extra_tokens: bool,
        encoding: Option<String>,
        state: PyRwLock<PyTokenizerIterState>,
    }

    impl PyTokenizerIter {
        fn readline(&self, vm: &VirtualMachine) -> PyResult<String> {
            // TODO: When `readline` is PyObject,
            // we need to check if it's callable and raise a type error if it's not.
            let raw_line = match self.readline.invoke((), vm) {
                Ok(v) => v,
                Err(_) => return Ok(String::new()),
            };
            Ok(match &self.encoding {
                Some(encoding) => {
                    let bytes = raw_line
                        .downcast::<PyBytes>()
                        .map_err(|_| vm.new_type_error("readline() returned a non-bytes object"))?;
                    vm.state
                        .codec_registry
                        .decode_text(bytes.into(), encoding, None, vm)
                        .map(|s| s.as_str().to_owned())?
                }
                None => raw_line
                    .downcast::<PyStr>()
                    .map(|s| s.as_str().to_owned())
                    .map_err(|_| vm.new_type_error("readline() returned a non-string object"))?,
            })
        }
    }

    impl fmt::Debug for PyTokenizerIter {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("PyTokenizerIter")
                .field("readline", &self.readline)
                .field("encoding", &self.encoding)
                .field("extra_tokens", &self.extra_tokens)
                .finish()
        }
    }

    #[pyclass(with(Constructor, Iterable, IterNext))]
    impl PyTokenizerIter {}

    impl Constructor for PyTokenizerIter {
        type Args = PyTokenizerIterArgs;

        fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            let Self::Args {
                readline,
                extra_tokens,
                encoding,
            } = args;

            Self {
                readline,
                extra_tokens,
                encoding: encoding.map(|s| s.as_str().to_owned()),
                state: PyRwLock::new(PyTokenizerIterState::default()),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    impl SelfIter for PyTokenizerIter {}

    impl IterNext for PyTokenizerIter {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let mut state = {
                let guard = zelf.state.read();
                guard.clone()
            };

            if state.eof {
                return Ok(PyIterReturn::StopIteration(None));
            }

            let token = loop {
                // TODO: Check here for errors. Raise SyntaxError if needed

                if let Some(tok) = state.next_token() {
                    break tok;
                }

                let nline = zelf.readline(vm)?;
                if nline.is_empty() {
                    state.eof = true;
                    *zelf.state.write() = state.clone();

                    let line_num = &state.start().0;
                    let out = vm
                        .ctx
                        .new_tuple(vec![
                            token_kind_value(TokenKind::EndOfFile).to_pyobject(vm),
                            vm.ctx.new_str("").into(),
                            vm.ctx
                                .new_tuple(vec![line_num.to_pyobject(vm), (-1).to_pyobject(vm)])
                                .into(),
                            vm.ctx
                                .new_tuple(vec![line_num.to_pyobject(vm), (-1).to_pyobject(vm)])
                                .into(),
                            vm.ctx.new_str(state.current_line()).into(),
                        ])
                        .into();
                    return Ok(PyIterReturn::Return(out));
                }
                state.push_line(&nline);
            };

            *zelf.state.write() = state.clone();

            let token_kind = token.kind();
            let token_value = if zelf.extra_tokens && token_kind.is_operator() {
                TOKEN_OP
            } else {
                token_kind_value(token_kind)
            };
            let (start_x, start_y) = &state.start();
            let (end_x, end_y) = &state.end();

            let mut token_repr = &state.source[state.range()];
            if !zelf.extra_tokens {
                token_repr = token_repr.trim();
            }

            let out = vm
                .ctx
                .new_tuple(vec![
                    token_value.to_pyobject(vm),
                    vm.ctx.new_str(token_repr).into(),
                    vm.ctx
                        .new_tuple(vec![start_x.to_pyobject(vm), start_y.to_pyobject(vm)])
                        .into(),
                    vm.ctx
                        .new_tuple(vec![end_x.to_pyobject(vm), end_y.to_pyobject(vm)])
                        .into(),
                    vm.ctx.new_str(state.current_line()).into(),
                ])
                .into();
            Ok(PyIterReturn::Return(out))
        }
    }

    #[derive(FromArgs)]
    pub struct PyTokenizerIterArgs {
        #[pyarg(positional)]
        readline: ArgCallable,
        #[pyarg(named)]
        extra_tokens: bool,
        #[pyarg(named, optional)]
        encoding: Option<PyStrRef>,
    }

    #[derive(Clone, Debug)]
    struct PyTokenizerIterState {
        /// Source code.
        source: String,
        prev_token: Option<Token>,
        /// Tokens of `source`.
        tokens: Tokens,
        /// Errors of `source`
        errors: Vec<ParseError>,
        /// LineIndex of `source`.
        line_index: LineIndex,
        /// Marker that says we already emitted EOF, and needs to stop iterating.
        eof: bool,
    }

    impl PyTokenizerIterState {
        fn push_line(&mut self, line: &str) {
            self.source.push_str(line);

            let parsed = parse_unchecked_source(&self.source, PySourceType::Python);
            self.tokens = parsed.tokens().clone();
            self.errors = parsed.errors().to_vec();
            self.line_index = LineIndex::from_source_text(&self.source);
        }

        #[must_use]
        fn current_line(&self) -> &str {
            let (kind, range) = match self.prev_token {
                Some(token) => token.as_tuple(),
                None => (TokenKind::Unknown, TextRange::default()),
            };

            match kind {
                TokenKind::Newline => self.source.full_line_str(range.start()),
                _ => self.source.full_lines_str(range),
            }
        }

        #[must_use]
        fn next_token(&mut self) -> Option<Token> {
            for token in self.tokens.iter() {
                let (kind, range) = token.as_tuple();

                if matches!(kind, TokenKind::NonLogicalNewline) {
                    continue;
                }

                if matches!(range.ordering(self.range()), Ordering::Greater) {
                    self.prev_token = Some(*token);
                    return self.prev_token;
                }
            }

            None
        }

        #[must_use]
        fn range(&self) -> TextRange {
            match self.prev_token {
                Some(token) => token.range(),
                None => TextRange::default(),
            }
        }

        #[must_use]
        fn start(&self) -> (usize, usize) {
            let lc = self
                .line_index
                .line_column(self.range().start(), &self.source);
            (lc.line.get(), lc.column.to_zero_indexed())
        }

        #[must_use]
        fn end(&self) -> (usize, usize) {
            let lc = self
                .line_index
                .line_column(self.range().end(), &self.source);
            (lc.line.get(), lc.column.to_zero_indexed())
        }
    }

    impl Default for PyTokenizerIterState {
        fn default() -> Self {
            const SOURCE: &str = "";
            let parsed = parse_unchecked_source(SOURCE, PySourceType::Python);

            Self {
                source: SOURCE.to_owned(),
                prev_token: None,
                tokens: parsed.tokens().clone(),
                errors: parsed.errors().to_vec(),
                line_index: LineIndex::from_source_text(SOURCE),
                eof: false,
            }
        }
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
            TokenKind::Newline | TokenKind::NonLogicalNewline => 4,
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
            TokenKind::Comment => 62,
            TokenKind::TStringStart => 62,  // 3.14 compatible
            TokenKind::TStringMiddle => 63, // 3.14 compatible
            TokenKind::TStringEnd => 64,    // 3.14 compatible
            TokenKind::IpyEscapeCommand | TokenKind::Question => 0, // Ruff's specific
            TokenKind::Unknown => 0,
        }
    }
}
