use ruff_python_ast::token::{Token, TokenFlags, TokenKind};
use ruff_text_size::{Ranged, TextRange, TextSize};

use crate::Mode;
use crate::error::LexicalError;
use crate::lexer::{Lexer, LexerCheckpoint};
use crate::string::InterpolatedStringKind;

/// Token source for the parser that skips over any trivia tokens.
#[derive(Debug)]
pub(crate) struct TokenSource<'src> {
    /// The underlying source for the tokens.
    lexer: Lexer<'src>,

    /// A vector containing all the tokens emitted by the lexer. This is returned when the parser
    /// is finished consuming all the tokens. Note that unlike the emitted tokens, this vector
    /// holds both the trivia and non-trivia tokens.
    tokens: Vec<Token>,

    source: &'src str,
    /// Append-only within the committed branch; recovery never rewrites history.
    diagnostic_reads: Vec<crate::diagnostic_events::DiagnosticTokenRead>,
    diagnostic_origin: crate::diagnostic_events::DiagnosticReadOrigin,
    diagnostic_reads_active: bool,
    current_discoveries_start: usize,
    lexical_divergence: Option<crate::diagnostic_events::LexicalDivergence>,
    diagnostic_snapshot: Option<Box<crate::lexer::DiagnosticLexerSnapshot>>,
}

impl<'src> TokenSource<'src> {
    /// Create a new token source for the given lexer.
    fn new(lexer: Lexer<'src>, source: &'src str, start_offset: TextSize) -> Self {
        TokenSource {
            lexer,
            tokens: allocate_tokens_vec(&source[start_offset.to_usize()..]),
            source,
            diagnostic_reads: Vec::new(),
            diagnostic_origin: crate::diagnostic_events::DiagnosticReadOrigin::FirstPass,
            diagnostic_reads_active: false,
            current_discoveries_start: 0,
            lexical_divergence: None,
            diagnostic_snapshot: None,
        }
    }

    /// Create a new token source from the given source code which starts at the given offset.
    pub(crate) fn from_source(source: &'src str, mode: Mode, start_offset: TextSize) -> Self {
        let lexer = Lexer::new(source, mode, start_offset);
        let mut source = TokenSource::new(lexer, source, start_offset);

        // Initialize the token source so that the current token is set correctly.
        source.do_bump();
        source
    }

    /// Returns the kind of the current token.
    pub(crate) const fn current_kind(&self) -> TokenKind {
        self.lexer.current_kind()
    }

    /// Returns the range of the current token.
    pub(crate) const fn current_range(&self) -> TextRange {
        self.lexer.current_range()
    }

    /// Returns the flags for the current token.
    pub(crate) const fn current_flags(&self) -> TokenFlags {
        self.lexer.current_flags()
    }

    /// Original committed discoveries before the current token remain visible even
    /// if parser recovery later removes an error from the ordinary error vector.
    pub(crate) fn has_error_before_current(&self) -> bool {
        self.current_discoveries_start != 0
    }

    pub(crate) fn diagnostic_parser_point(&self) -> crate::diagnostic_events::DiagnosticParserPoint {
        crate::diagnostic_events::DiagnosticParserPoint {
            token: Token::new(self.current_kind(), self.current_range(), self.current_flags()),
            read: self.diagnostic_reads.last().map(|read| read.ordinal),
            original_lexical_prefix: self.lexer.diagnostic_discoveries().is_empty()
                && self.lexical_divergence.is_none(),
        }
    }

    /// A temporary parser validation error can be deferred as a valid generator
    /// operand. Restore only its observation marker, never lexer state or reads.
    pub(crate) fn diagnostic_parser_origin(&self) -> crate::diagnostic_events::DiagnosticReadOrigin {
        self.diagnostic_origin
    }

    pub(crate) fn restore_diagnostic_parser_origin(
        &mut self,
        origin: crate::diagnostic_events::DiagnosticReadOrigin,
    ) {
        self.diagnostic_origin = origin;
    }

    pub(crate) fn mark_parser_recovery(&mut self) {
        self.diagnostic_origin = crate::diagnostic_events::DiagnosticReadOrigin::ParserRecovery;
    }

    fn record_lexical_recovery(&mut self, effect: crate::diagnostic_events::LexerRecoveryEffect) {
        use crate::diagnostic_events::{LexerRecoveryEffect, LexicalDivergence};
        if effect != LexerRecoveryEffect::Unchanged {
            self.mark_parser_recovery();
            self.lexical_divergence.get_or_insert(LexicalDivergence {
                next_read: self.diagnostic_reads.len(),
                effect,
            });
        }
    }

    pub(crate) fn is_first_pass(&self) -> bool {
        self.diagnostic_origin == crate::diagnostic_events::DiagnosticReadOrigin::FirstPass
    }

    /// Begin only when an eligible header is found, seeding its current target.
    /// No-grammar inputs do not duplicate the token stream in a read journal.
    pub(crate) fn begin_diagnostic_reads(&mut self) -> usize {
        if !self.diagnostic_reads_active {
            self.diagnostic_reads_active = true;
            self.record_diagnostic_read(self.current_discoveries_start);
            if let Some(seed) = self.diagnostic_reads.last() {
                self.diagnostic_snapshot = Some(Box::new(self.lexer.diagnostic_snapshot(seed.clone())));
            }
        }
        self.diagnostic_reads.len().saturating_sub(1)
    }

    fn record_diagnostic_read(&mut self, discoveries_before: usize) {
        if cfg!(feature = "python315-diagnostics") && self.diagnostic_reads_active {
            let token = Token::new(self.current_kind(), self.current_range(), self.current_flags());
            let discoveries = self.lexer.diagnostic_discoveries()[discoveries_before..].to_vec();
            let read_range = (!discoveries.iter().any(|error| error.token_range.is_none()))
                .then(|| self.lexer.diagnostic_read_range());
            self.diagnostic_reads.push(crate::diagnostic_events::DiagnosticTokenRead {
                ordinal: self.diagnostic_reads.len(),
                token_slot: self.tokens.len(),
                read_range,
                origin: self.diagnostic_origin,
                discoveries,
                warnings: self.lexer.diagnostic_warnings().to_vec(),
                type_comment: crate::diagnostic_events::classify_type_comment(self.source, &token),
                end_of_input: token.kind() == TokenKind::EndOfFile,
                token,
            });
        }
    }

    /// Returns `true` if the lexer found an error that stops tokenization in `range`.
    pub(crate) fn has_tokenizer_error_in(&self, range: TextRange) -> bool {
        self.lexer.errors().iter().any(|error| {
            range.contains_inclusive(error.location().start()) && error.error().is_tokenizer_error()
        })
    }

    /// Calls the underlying [`re_lex_logical_token`] method on the lexer with the new lexer
    /// position and updates the token vector accordingly.
    ///
    /// [`re_lex_logical_token`]: Lexer::re_lex_logical_token
    pub(crate) fn re_lex_logical_token(&mut self) -> crate::diagnostic_events::LexerRecoveryEffect {
        let mut non_logical_newline = None;

        #[cfg(debug_assertions)]
        let last_non_trivia_end_before = {
            self.tokens
                .iter()
                .rev()
                .find(|tok| !tok.kind().is_trivia())
                .map(ruff_text_size::Ranged::end)
        };

        for (index, token) in self.tokens.iter().enumerate().rev() {
            match token.kind() {
                TokenKind::NonLogicalNewline => {
                    non_logical_newline = Some((index, token.start()));
                }
                TokenKind::Comment => continue,
                _ => break,
            }
        }

        let discoveries_before = self.lexer.diagnostic_discoveries().len();
        let effect = self.lexer.re_lex_logical_token(non_logical_newline.map(|(_, start)| start));
        self.record_lexical_recovery(effect);
        if effect != crate::diagnostic_events::LexerRecoveryEffect::Retokenized {
            return effect;
        }

        let non_logical_line_index = non_logical_newline
            .expect(
                "retokenized logical recovery requires a non-logical newline",
            )
            .0;

        // Trim the already bumped logical line token (and comments coming after it) as it might now have become a logical line token
        self.tokens.truncate(non_logical_line_index);
        // The direct lexer.next_token inside re-lex is a new recovery-origin read.
        // Keep earlier original reads even though final token slots are reused.
        self.current_discoveries_start = discoveries_before;
        self.record_diagnostic_read(discoveries_before);

        #[cfg(debug_assertions)]
        {
            let last_non_trivia_end_now = {
                self.tokens
                    .iter()
                    .rev()
                    .find(|tok| !tok.kind().is_trivia())
                    .map(ruff_text_size::Ranged::end)
            };

            assert_eq!(last_non_trivia_end_before, last_non_trivia_end_now);
        }

        // Ensure `current` is positioned at a non-trivia token.
        if self.current_kind().is_trivia() {
            self.bump(self.current_kind());
        }
        effect
    }

    pub(crate) fn re_lex_string_token_in_interpolation_element(
        &mut self,
        kind: InterpolatedStringKind,
    ) {
        let discoveries_before = self.lexer.diagnostic_discoveries().len();
        let effect = self.lexer.re_lex_string_token_in_interpolation_element(kind);
        self.record_lexical_recovery(effect);
        if effect == crate::diagnostic_events::LexerRecoveryEffect::TokenRewritten {
            self.current_discoveries_start = discoveries_before;
            self.record_diagnostic_read(discoveries_before);
        }
    }

    pub(crate) fn re_lex_raw_string_in_format_spec(&mut self) {
        let discoveries_before = self.lexer.diagnostic_discoveries().len();
        let effect = self.lexer.re_lex_raw_string_in_format_spec();
        self.record_lexical_recovery(effect);
        if effect == crate::diagnostic_events::LexerRecoveryEffect::TokenRewritten {
            self.current_discoveries_start = discoveries_before;
            self.record_diagnostic_read(discoveries_before);
        }
    }

    /// Returns the next non-trivia token without consuming it.
    ///
    /// Use [`peek2`] to get the next two tokens.
    ///
    /// [`peek2`]: TokenSource::peek2
    pub(crate) fn peek(&mut self) -> TokenKind {
        let checkpoint = self.lexer.checkpoint();
        let next = self.next_non_trivia_token();
        self.lexer.rewind(checkpoint);
        next
    }

    /// Returns the next two non-trivia tokens without consuming it.
    ///
    /// Use [`peek`] to only get the next token.
    ///
    /// [`peek`]: TokenSource::peek
    pub(crate) fn peek2(&mut self) -> (TokenKind, TokenKind) {
        let checkpoint = self.lexer.checkpoint();
        let first = self.next_non_trivia_token();
        let second = self.next_non_trivia_token();
        self.lexer.rewind(checkpoint);
        (first, second)
    }

    /// Bumps the token source to the next non-trivia token.
    ///
    /// It pushes the given kind to the token vector with the current token range.
    pub(crate) fn bump(&mut self, kind: TokenKind) {
        if kind == TokenKind::Unknown { self.mark_parser_recovery(); }
        self.tokens
            .push(Token::new(kind, self.current_range(), self.current_flags()));
        self.do_bump();
    }

    /// Bumps the token source to the next non-trivia token without adding the current token to the
    /// token vector. It does add the trivia tokens to the token vector.
    fn do_bump(&mut self) {
        loop {
            let discoveries_before = self.lexer.diagnostic_discoveries().len();
            let kind = self.lexer.next_token();
            self.current_discoveries_start = discoveries_before;
            self.record_diagnostic_read(discoveries_before);
            if kind.is_trivia() {
                self.tokens
                    .push(Token::new(kind, self.current_range(), self.current_flags()));
                continue;
            }
            break;
        }
    }

    /// Returns the next non-trivia token without adding it to the token vector.
    fn next_non_trivia_token(&mut self) -> TokenKind {
        loop {
            let kind = self.lexer.next_token();
            if kind.is_trivia() {
                continue;
            }
            break kind;
        }
    }

    /// Creates a checkpoint to which the token source can later return to using [`Self::rewind`].
    pub(crate) fn checkpoint(&self) -> TokenSourceCheckpoint {
        TokenSourceCheckpoint {
            lexer_checkpoint: self.lexer.checkpoint(),
            tokens_position: self.tokens.len(),
            discoveries_position: self.diagnostic_reads.len(),
            diagnostic_origin: self.diagnostic_origin,
            diagnostic_reads_active: self.diagnostic_reads_active,
            current_discoveries_start: self.current_discoveries_start,
            lexical_divergence: self.lexical_divergence,
            had_diagnostic_snapshot: self.diagnostic_snapshot.is_some(),
        }
    }

    /// Restore the token source to the given checkpoint.
    pub(crate) fn rewind(&mut self, checkpoint: TokenSourceCheckpoint) {
        let TokenSourceCheckpoint {
            lexer_checkpoint,
            tokens_position,
            discoveries_position,
            diagnostic_origin,
            diagnostic_reads_active,
            current_discoveries_start,
            lexical_divergence,
            had_diagnostic_snapshot,
        } = checkpoint;

        self.lexer.rewind(lexer_checkpoint);
        self.tokens.truncate(tokens_position);
        self.diagnostic_reads.truncate(discoveries_position);
        self.diagnostic_origin = diagnostic_origin;
        self.diagnostic_reads_active = diagnostic_reads_active;
        self.current_discoveries_start = current_discoveries_start;
        self.lexical_divergence = lexical_divergence;
        if !had_diagnostic_snapshot { self.diagnostic_snapshot = None; }
    }

    /// Returns the tokens bumped so far, including the trivia tokens.
    pub(crate) fn bumped(&self) -> &[Token] {
        &self.tokens
    }

    /// Returns a slice of [`Token`] that are within the given `range`.
    pub(crate) fn in_range(&self, range: TextRange) -> &[Token] {
        let start = self
            .tokens
            .iter()
            .rposition(|tok| tok.start() == range.start());
        let end = self.tokens.iter().rposition(|tok| tok.end() == range.end());

        let (Some(start), Some(end)) = (start, end) else {
            return &self.tokens;
        };

        &self.tokens[start..=end]
    }

    /// Consumes the token source, returning the collected tokens, comment ranges, and any errors
    /// encountered during lexing. The token collection includes both the trivia and non-trivia
    /// tokens.
    pub(crate) fn finish(mut self) -> (
        Vec<Token>, Vec<LexicalError>, Vec<crate::diagnostic_events::DiagnosticTokenRead>,
        Option<crate::diagnostic_events::LexicalDivergence>,
        Option<Box<crate::lexer::DiagnosticLexerSnapshot>>,
    ) {
        assert_eq!(
            self.current_kind(),
            TokenKind::EndOfFile,
            "TokenSource was not fully consumed"
        );

        // The `EndOfFile` token shouldn't be included in the token stream, it's mainly to signal
        // the parser to stop. This isn't in `do_bump` because it only needs to be done once.
        if let Some(last) = self.tokens.pop() {
            assert_eq!(last.kind(), TokenKind::EndOfFile);
        }

        self.tokens.shrink_to_fit();

        (self.tokens, self.lexer.finish(), self.diagnostic_reads, self.lexical_divergence, self.diagnostic_snapshot)
    }
}

pub(crate) struct TokenSourceCheckpoint {
    lexer_checkpoint: LexerCheckpoint,
    tokens_position: usize,
    discoveries_position: usize,
    diagnostic_origin: crate::diagnostic_events::DiagnosticReadOrigin,
    diagnostic_reads_active: bool,
    current_discoveries_start: usize,
    lexical_divergence: Option<crate::diagnostic_events::LexicalDivergence>,
    had_diagnostic_snapshot: bool,
}

/// Allocates a token buffer with a capacity intended to skip early grows.
fn allocate_tokens_vec(contents: &str) -> Vec<Token> {
    // In sampled ruff-ecosystem projects, about three quarters of Python files contain at least
    // one token per eight source bytes. Intentionally underestimate the final token count to avoid
    // over-reserving for token-sparse files.
    const BYTES_PER_PREALLOCATED_TOKEN: usize = 8;
    const MIN_INITIAL_CAPACITY: usize = 128;

    let capacity_hint = contents.len() / BYTES_PER_PREALLOCATED_TOKEN;
    if capacity_hint < MIN_INITIAL_CAPACITY {
        return Vec::new();
    }

    // Stay on a power-of-two bucket so that later geometric growth does not preserve an
    // arbitrary capacity offset.
    Vec::with_capacity(capacity_hint.isolate_highest_one())
}
