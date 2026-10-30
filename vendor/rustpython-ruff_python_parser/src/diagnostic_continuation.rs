//! A conservative consumer view of original tokenizer observations.
//!
//! This does not emit warnings, choose a Python exception, infer parser
//! reachability, or reconstruct a suffix lost to parser-requested lexer recovery.

use crate::diagnostic_events::{
    DiagnosticContinuity, DiagnosticTokenRead, LexicalDiscovery, LexicalDisposition, LexicalDivergence,
};

#[derive(Debug)]
pub enum ContinuationTermination<'a> {
    IndependentStreams,
    EndOfInput { read: &'a DiagnosticTokenRead },
    HardStop {
        read: &'a DiagnosticTokenRead,
        discovery: &'a LexicalDiscovery,
    },
    UnclassifiedDiscovery {
        read: &'a DiagnosticTokenRead,
        discovery: &'a LexicalDiscovery,
    },
    Diverged(LexicalDivergence),
    MissingRead { ordinal: usize },
    UnboundFrontier { ordinal: usize },
    FailureAlreadyRead {
        read: &'a DiagnosticTokenRead,
        discovery: &'a LexicalDiscovery,
    },
}

#[derive(Debug)]
pub struct OriginalContinuation<'a> {
    /// Original-lineage observations strictly after the caller's filled frontier.
    /// A terminal failed read is included. Events inside it must still respect
    /// the discovery cursor; its complete recovered token span is not permission
    /// to emit everything within that span.
    pub reads: &'a [DiagnosticTokenRead],
    pub termination: ContinuationTermination<'a>,
}

/// Inspect original lexical lineage after a proven completed action's read frontier.
/// ParserRecovery alone is not a lexer rewrite and does not truncate this view.
/// A divergence is incomplete evidence, never successful exhaustion.
///
/// The caller must separately establish TYPE_COMMENTS and Single reachability,
/// pending exception identity, and CPython's furthest-filled-token START line.
/// This view does not promote unknown lexical mappings into canonical hard stops.
pub fn original_continuation_after(
    reads: &[DiagnosticTokenRead],
    continuity: DiagnosticContinuity,
    frontier: usize,
) -> OriginalContinuation<'_> {
    let empty = &reads[..0];
    let DiagnosticContinuity::SingleStream { divergence } = continuity else {
        return OriginalContinuation {
            reads: empty,
            termination: ContinuationTermination::IndependentStreams,
        };
    };
    let Some(filled) = reads.get(frontier).filter(|read| read.ordinal == frontier) else {
        return OriginalContinuation {
            reads: empty,
            termination: ContinuationTermination::UnboundFrontier { ordinal: frontier },
        };
    };
    if let Some(divergence) = divergence.filter(|value| value.next_read <= frontier) {
        return OriginalContinuation {
            reads: empty,
            termination: ContinuationTermination::Diverged(divergence),
        };
    }
    // A claimed filled frontier cannot silently cross an already discovered
    // stop, a missing original token, or a non-contiguous read ordinal.
    for (index, read) in reads[..=frontier].iter().enumerate() {
        if let Some(discovery) = read.discoveries.iter().find(|discovery| {
            !discovery.eof_cleanup && discovery.disposition != LexicalDisposition::ParserToken
        }) {
            return OriginalContinuation {
                reads: empty,
                termination: ContinuationTermination::FailureAlreadyRead { read, discovery },
            };
        }
        if read.ordinal != index || read.read_range.is_none() {
            return OriginalContinuation {
                reads: empty,
                termination: ContinuationTermination::UnboundFrontier { ordinal: frontier },
            };
        }
    }
    if filled.end_of_input {
        return OriginalContinuation {
            reads: empty,
            termination: ContinuationTermination::EndOfInput { read: filled },
        };
    }
    let start = frontier + 1;
    for (index, read) in reads.iter().enumerate().skip(start) {
        if let Some(divergence) = divergence.filter(|value| value.next_read <= index) {
            return OriginalContinuation {
                reads: &reads[start..index],
                termination: ContinuationTermination::Diverged(divergence),
            };
        }
        if read.ordinal != index {
            return OriginalContinuation {
                reads: &reads[start..index],
                termination: ContinuationTermination::MissingRead { ordinal: index },
            };
        }
        if let Some(discovery) = read.discoveries.iter().find(|discovery| {
            !discovery.eof_cleanup && discovery.disposition != LexicalDisposition::ParserToken
        }) {
            let termination = match discovery.disposition {
                LexicalDisposition::BareErrorToken | LexicalDisposition::RaisedErrorToken => {
                    ContinuationTermination::HardStop { read, discovery }
                }
                LexicalDisposition::DecoderOrUnclassified => {
                    ContinuationTermination::UnclassifiedDiscovery { read, discovery }
                }
                LexicalDisposition::ParserToken => unreachable!(),
            };
            return OriginalContinuation {
                reads: &reads[start..=index],
                termination,
            };
        }
        if read.read_range.is_none() {
            return OriginalContinuation {
                reads: &reads[start..index],
                termination: ContinuationTermination::MissingRead { ordinal: index },
            };
        }
        if read.end_of_input {
            return OriginalContinuation {
                reads: &reads[start..=index],
                termination: ContinuationTermination::EndOfInput { read },
            };
        }
    }
    let termination = match divergence.filter(|value| value.next_read <= reads.len()) {
        Some(divergence) => ContinuationTermination::Diverged(divergence),
        None => ContinuationTermination::MissingRead { ordinal: reads.len() },
    };
    OriginalContinuation { reads: &reads[start..], termination }
}
