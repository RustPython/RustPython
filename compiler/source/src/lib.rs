pub use ruff_source_file::{LineIndex, OneIndexed as LineNumber, SourceLocation};
use ruff_text_size::TextRange;
pub use ruff_text_size::TextSize;

#[derive(Clone)]
pub struct SourceCode<'src> {
    pub path: &'src str,
    pub text: &'src str,
    pub index: LineIndex,
}

impl<'src> SourceCode<'src> {
    pub fn new(path: &'src str, text: &'src str) -> Self {
        let index = LineIndex::from_source_text(text);
        Self { path, text, index }
    }

    pub fn line_index(&self, offset: TextSize) -> LineNumber {
        self.index.line_index(offset)
    }

    pub fn source_location(&self, offset: TextSize) -> SourceLocation {
        self.index.source_location(offset, self.text)
    }

    pub fn get_range(&'src self, range: TextRange) -> &'src str {
        &self.text[range.start().to_usize()..range.end().to_usize()]
    }
}

pub struct SourceCodeOwned {
    pub path: String,
    pub text: String,
    pub index: LineIndex,
}

impl SourceCodeOwned {
    pub fn new(path: String, text: String) -> Self {
        let index = LineIndex::from_source_text(&text);
        Self { path, text, index }
    }
}
