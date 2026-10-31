//! CPython's first-pass `import_from_targets` action boundary.
//!
//! Only the three successful target forms are recognized. Consumed targets,
//! required lookahead, TYPE_COMMENT reachability and diagnostics stay distinct.

use ruff_python_ast::token::TokenKind;
use ruff_text_size::{Ranged, TextRange, TextSize};
use crate::diagnostic_events::{DiagnosticTokenRead, DiagnosticReadOrigin, LexicalDisposition, TypeCommentKind, TypeCommentToken};

/// Target-local action provenance, not a complete canonical parser trace.
/// A consumer must still enforce earlier TYPE_COMMENT validation and Single
/// completion/recovery boundaries before emitting a warning.
#[derive(Debug, PartialEq, Eq, Clone, get_size2::GetSize)]
pub struct RelativeLazyImportWarning {
    pub module: TextRange,
    pub level: u32,
    pub targets_end: TextSize,
    /// Committed read ordinal, not a final recovered token-buffer index.
    pub tokenizer_read: usize,
    pub tokenizer_start: TextSize,
    pub tokenizer_end: TextSize,
    /// A bare-list negative-comma lookahead may read a TYPE_COMMENT while the
    /// action still completes. Parenthesized targets cannot skip this token.
    pub type_comment_lookahead: Option<TypeCommentToken>,
}

pub(crate) fn complete(
    candidates: Vec<(TextRange, u32, usize)>,
    reads: &[DiagnosticTokenRead],
    type_comments: bool,
) -> Vec<RelativeLazyImportWarning> {
    let mut warnings = Vec::new();
    for (module, level, start) in candidates {
        let Some(first) = reads.get(start) else { continue; };
        let Some(first_range) = first.read_range else { continue; };
        let targets_start = first_range.start();
        let mut targets = Targets {
            reads: &reads[start..], type_comments, position: 0,
            consumed_end: targets_start, read_range: TextRange::empty(targets_start),
            read_index: 0, type_comment_lookahead: None, blocked_lookahead: false,
        };
        if targets.parse()
            && !targets.blocked_lookahead
            && !reads[start..=start + targets.read_index].iter().any(|read| {
                read.origin != DiagnosticReadOrigin::FirstPass
                    || read.discoveries.iter().any(|error| {
                        !error.eof_cleanup && matches!(error.disposition,
                            LexicalDisposition::BareErrorToken | LexicalDisposition::RaisedErrorToken)
                    })
            })
        {
            warnings.push(RelativeLazyImportWarning {
                module, level, targets_end: targets.consumed_end,
                tokenizer_read: start + targets.read_index,
                tokenizer_start: targets.read_range.start(),
                tokenizer_end: targets.read_range.end(),
                type_comment_lookahead: targets.type_comment_lookahead,
            });
        }
    }
    warnings
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TargetToken { Syntax(TokenKind), TypeComment }

struct Targets<'a> {
    reads: &'a [DiagnosticTokenRead],
    type_comments: bool,
    position: usize,
    consumed_end: TextSize,
    read_range: TextRange,
    read_index: usize,
    type_comment_lookahead: Option<TypeCommentToken>,
    /// Failed speculative lookahead is not undone by optional-as rollback.
    blocked_lookahead: bool,
}

impl Targets<'_> {
    fn peek(&mut self) -> Option<TargetToken> {
        loop {
            let Some(read) = self.reads.get(self.position) else {
                self.blocked_lookahead = true;
                return None;
            };
            if read.origin != DiagnosticReadOrigin::FirstPass || read.read_range.is_none() {
                self.read_index = self.read_index.max(self.position);
                self.blocked_lookahead = true;
                return None;
            }
            let read_range = read.read_range.unwrap();
            if self.position >= self.read_index {
                self.read_index = self.position;
                self.read_range = read_range;
                self.type_comment_lookahead = None;
            }
            if self.type_comments
                && let Some(comment) = read.type_comment
                && comment.kind == TypeCommentKind::Regular
            {
                if self.position == self.read_index { self.type_comment_lookahead = Some(comment); }
                return Some(TargetToken::TypeComment);
            }
            if read.token.kind().is_trivia() {
                self.position += 1;
                continue;
            }
            return Some(TargetToken::Syntax(read.token.kind()));
        }
    }

    fn take(&mut self) {
        self.consumed_end = self.reads[self.position].token.end();
        self.position += 1;
    }

    fn eat(&mut self, kind: TokenKind) -> bool {
        if self.peek() == Some(TargetToken::Syntax(kind)) {
            self.take(); true
        } else { false }
    }

    fn name(&mut self) -> bool {
        if self.peek().is_some_and(|token| matches!(token, TargetToken::Syntax(kind)
            if kind == TokenKind::Identifier || kind.is_soft_keyword())) {
            self.take(); true
        } else { false }
    }

    fn alias(&mut self) -> bool {
        if !self.name() { return false; }
        let checkpoint = (self.position, self.consumed_end);
        if self.eat(TokenKind::As) && !self.name() {
            (self.position, self.consumed_end) = checkpoint;
        }
        true
    }

    fn aliases(&mut self) -> bool {
        if !self.alias() { return false; }
        loop {
            let checkpoint = (self.position, self.consumed_end);
            if !self.eat(TokenKind::Comma) { return true; }
            if !self.alias() {
                (self.position, self.consumed_end) = checkpoint;
                return true;
            }
        }
    }

    fn parse(&mut self) -> bool {
        if self.eat(TokenKind::Lpar) {
            if !self.aliases() { return false; }
            self.eat(TokenKind::Comma);
            self.eat(TokenKind::Rpar)
        } else if self.aliases() {
            // The negative-comma lookahead must actually be available. A
            // recovery boundary is not proof of a successfully read token.
            self.peek().is_some_and(|token| token != TargetToken::Syntax(TokenKind::Comma))
        } else {
            self.eat(TokenKind::Star)
        }
    }
}
