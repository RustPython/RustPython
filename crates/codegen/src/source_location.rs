//! Explicit locations supplied by Python AST objects.
//!
//! Parser input continues to resolve its text ranges through the source file.
//! AST input needs node identity because absent columns and a real column zero
//! can produce the same normalized text range.

use ruff_python_ast::HasNodeIndex;
use ruff_text_size::TextRange;
use rustpython_compiler_core::{AstLocations, SourceLocationRange};

/// NONE is a shared placeholder and can never identify semantic coordinates.
pub(crate) fn node_location(
    locations: &AstLocations,
    node: &impl HasNodeIndex,
) -> Option<SourceLocationRange> {
    locations.get(node.node_index().load().as_u32()?)
}

/// Navigation range and semantic coordinates must be saved/restored together.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CompilerSourceSpan {
    pub range: TextRange,
    pub location: Option<SourceLocationRange>,
}

impl CompilerSourceSpan {
    pub(crate) fn is_artificial(self) -> bool {
        self.location
            .map_or(self.range == TextRange::default(), |location| {
                location.line < 0
            })
    }

    pub(crate) const fn parsed(range: TextRange) -> Self {
        Self {
            range,
            location: None,
        }
    }
}
