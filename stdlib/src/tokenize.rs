pub(crate) use _tokenize::make_module;

#[pymodule]
mod _tokenize {
    use crate::vm::{
        Py, PyPayload, PyResult, VirtualMachine,
        builtins::{PyStr, PyTypeRef},
        convert::ToPyObject,
        function::ArgCallable,
        protocol::PyIterReturn,
        types::{Constructor, IterNext, Iterable, SelfIter},
    };
    use ruff_python_ast::PySourceType;
    use ruff_python_parser::{TokenKind, Tokens, parse_unchecked_source};
    use ruff_source_file::{LineIndex, LineRanges};
    use std::{cmp::Ordering, fmt, sync::atomic};

    #[pyattr]
    #[pyclass(name = "TokenizerIter")]
    #[derive(PyPayload)]
    pub struct PyTokenizerIter {
        source: String,
        tokens: Tokens,
        token_count: usize,
        token_idx: atomic::AtomicUsize,
        line_index: LineIndex,
    }
    impl PyTokenizerIter {
        fn bump_token_idx(&self) {
            let _ = self.token_idx.fetch_update(
                atomic::Ordering::SeqCst,
                atomic::Ordering::SeqCst,
                |x| Some(x + 1),
            );
        }
    }

    impl fmt::Debug for PyTokenizerIter {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("PyTokenizerIter")
                .field("source", &self.source)
                .field("tokens", &self.tokens)
                .field("token_idx", &self.token_idx)
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
                extra_tokens: _,
                encoding: _,
            } = args;

            // TODO: We should get the source lazily. But Ruff API doesn't really work as expected
            // when dealing with incomplete source code.
            // See: https://github.com/astral-sh/ruff/pull/21074
            let mut source = String::new();
            loop {
                // TODO: Downcast to diffrent type based on encoding.
                let line = readline
                    .invoke((), vm)?
                    .downcast::<PyStr>()
                    .map_err(|_| vm.new_type_error("readline() returned a non-string object"))?;

                if line.is_empty() {
                    break;
                }
                source.push_str(line.as_str());
            }

            let line_index = LineIndex::from_source_text(&source);
            let parsed = parse_unchecked_source(&source, PySourceType::Python);
            let tokens = parsed.tokens();

            Self {
                source,
                tokens: tokens.clone(),
                line_index,
                token_count: tokens.len(),
                token_idx: atomic::AtomicUsize::new(0),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    impl SelfIter for PyTokenizerIter {}

    impl IterNext for PyTokenizerIter {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let idx = zelf.token_idx.load(atomic::Ordering::SeqCst);

            let source = &zelf.source;
            let line_index = &zelf.line_index;
            Ok(match zelf.tokens.get(idx) {
                Some(token) => {
                    zelf.bump_token_idx();

                    let (token_kind, token_range) = token.as_tuple();
                    let lc_start = line_index.line_column(token_range.start(), &source);
                    let lc_end = line_index.line_column(token_range.end(), &source);

                    let current_line = match token_kind {
                        TokenKind::Newline => source.full_line_str(token_range.start()),
                        _ => source.full_lines_str(token_range),
                    };

                    let out = vm
                        .ctx
                        .new_tuple(vec![
                            token_kind_value(token_kind).to_pyobject(vm),
                            vm.ctx.new_str(source[token_range].trim()).into(),
                            vm.ctx
                                .new_tuple(vec![
                                    lc_start.line.get().to_pyobject(vm),
                                    lc_start.column.to_zero_indexed().to_pyobject(vm),
                                ])
                                .into(),
                            vm.ctx
                                .new_tuple(vec![
                                    lc_end.line.get().to_pyobject(vm),
                                    lc_end.column.to_zero_indexed().to_pyobject(vm),
                                ])
                                .into(),
                            vm.ctx.new_str(current_line).into(),
                        ])
                        .into();

                    PyIterReturn::Return(out)
                }
                None => {
                    match idx.cmp(&zelf.token_count) {
                        Ordering::Less => unreachable!(),
                        Ordering::Equal => {
                            zelf.bump_token_idx();
                            // TODO: EOF output
                            PyIterReturn::StopIteration(None)
                        }
                        Ordering::Greater => PyIterReturn::StopIteration(None),
                    }
                }
            })
        }
    }

    #[allow(dead_code)]
    #[derive(FromArgs)]
    pub struct PyTokenizerIterArgs {
        #[pyarg(positional)]
        readline: ArgCallable,
        #[pyarg(named)]
        extra_tokens: bool,
        #[pyarg(named, default = String::from("utf-8"))]
        encoding: String,
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
