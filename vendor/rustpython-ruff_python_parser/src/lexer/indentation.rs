use static_assertions::assert_eq_size;
use std::cmp::Ordering;
use std::fmt::Debug;

use ruff_python_trivia::tab_offset_u32;

/// The column index of an indentation.
///
/// A space increments the column by one. A tab advances the column to the next multiple of 8.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Default)]
pub(super) struct Column(u32);

impl Column {
    const fn new(column: u32) -> Self {
        Self(column)
    }
}

/// The number of characters in an indentation. Each character accounts for 1.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Default)]
pub(super) struct Character(u32);

impl Character {
    const fn new(characters: u32) -> Self {
        Self(characters)
    }
}

/// The [Indentation](https://docs.python.org/3/reference/lexical_analysis.html#indentation) of a logical line.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub(super) struct Indentation {
    column: Column,
    character: Character,
}

impl Indentation {
    const TAB_SIZE: u32 = 8;

    pub(super) const fn root() -> Self {
        Self {
            column: Column::new(0),
            character: Character::new(0),
        }
    }

    #[cfg(test)]
    const fn new(column: Column, character: Character) -> Self {
        Self { column, character }
    }

    #[must_use]
    pub(super) fn add_space(self) -> Self {
        Self {
            character: Character(self.character.0 + 1),
            column: Column(self.column.0 + 1),
        }
    }

    #[must_use]
    pub(super) fn add_tab(self) -> Self {
        Self {
            character: Character(self.character.0 + 1),
            // Compute the column index:
            // * Adds `TAB_SIZE` if `column` is a multiple of `TAB_SIZE`
            // * Rounds `column` up to the next multiple of `TAB_SIZE` otherwise.
            // https://github.com/python/cpython/blob/2cf99026d6320f38937257da1ab014fc873a11a6/Parser/tokenizer.c#L1818
            column: Column(self.column.0 + tab_offset_u32(self.column.0, Self::TAB_SIZE)),
        }
    }

    /// Compares the current indentation `self` with the indentation `other` of a new line by
    /// column.
    ///
    /// A deeper line must also have more characters, and a line at the same column the same
    /// number of characters; otherwise tabs and spaces are mixed inconsistently. A shallower line
    /// is checked against the level it dedents to, in [`Indentations::dedent_one`].
    pub(super) fn try_compare(self, other: Indentation) -> Result<Ordering, IndentationError> {
        let ordering = self.column.cmp(&other.column);
        let consistent = match ordering {
            Ordering::Less => self.character < other.character,
            Ordering::Equal => self.character == other.character,
            Ordering::Greater => true,
        };

        if consistent {
            Ok(ordering)
        } else {
            Err(IndentationError::InconsistentTabs)
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub(super) enum IndentationError {
    /// Tabs and spaces are mixed in a way that makes the indentation depend on the tab size.
    InconsistentTabs,
    /// A dedent does not match any outer indentation level.
    UnmatchedDedent,
    /// Indenting would exceed [`Indentations::MAX_DEPTH`].
    TooDeep,
}

/// The indentations stack is used to keep track of the current indentation level
/// [See Indentation](docs.python.org/3/reference/lexical_analysis.html#indentation).
#[derive(Debug, Clone, Default)]
pub(super) struct Indentations {
    stack: Vec<Indentation>,
}

impl Indentations {
    /// The maximum number of nested indentation levels.
    const MAX_DEPTH: usize = 99;

    pub(super) fn indent(&mut self, indent: Indentation) -> Result<(), IndentationError> {
        debug_assert_eq!(self.current().try_compare(indent), Ok(Ordering::Less));

        if self.stack.len() >= Self::MAX_DEPTH {
            return Err(IndentationError::TooDeep);
        }
        self.stack.push(indent);
        Ok(())
    }

    /// Dedent one level to eventually reach `new_indentation`.
    ///
    /// Returns `Err` if the `new_indentation` is greater than the new current indentation level.
    pub(super) fn dedent_one(
        &mut self,
        new_indentation: Indentation,
    ) -> Result<Option<Indentation>, IndentationError> {
        let previous = self.dedent();
        let current = *self.current();

        match new_indentation.column.cmp(&current.column) {
            Ordering::Less => Ok(previous),
            Ordering::Equal if new_indentation.character == current.character => Ok(previous),
            Ordering::Equal => Err(IndentationError::InconsistentTabs),
            // ```python
            // if True:
            //     pass
            //   pass <- The indentation is greater than the expected indent of 0.
            // ```
            Ordering::Greater => Err(IndentationError::UnmatchedDedent),
        }
    }

    pub(super) fn dedent(&mut self) -> Option<Indentation> {
        self.stack.pop()
    }

    pub(super) fn current(&self) -> &Indentation {
        static ROOT: Indentation = Indentation::root();
        self.stack.last().unwrap_or(&ROOT)
    }

    pub(crate) fn checkpoint(&self) -> IndentationsCheckpoint {
        IndentationsCheckpoint(self.stack.clone())
    }

    pub(crate) fn rewind(&mut self, checkpoint: IndentationsCheckpoint) {
        self.stack = checkpoint.0;
    }
}

#[derive(Debug, Clone)]
pub(crate) struct IndentationsCheckpoint(Vec<Indentation>);

assert_eq_size!(Indentation, u64);

#[cfg(test)]
mod tests {
    use super::{Character, Column, Indentation, IndentationError};
    use std::cmp::Ordering;

    #[test]
    fn indentation_try_compare() {
        let tab = Indentation::new(Column::new(8), Character::new(1));

        assert_eq!(tab.try_compare(tab), Ok(Ordering::Equal));

        let two_tabs = Indentation::new(Column::new(16), Character::new(2));
        assert_eq!(two_tabs.try_compare(tab), Ok(Ordering::Greater));
        assert_eq!(tab.try_compare(two_tabs), Ok(Ordering::Less));
    }

    #[test]
    fn indentation_try_compare_mixed_tabs() {
        let tab = Indentation::new(Column::new(8), Character::new(1));
        let eight_spaces = Indentation::new(Column::new(8), Character::new(8));
        let tab_and_space = Indentation::new(Column::new(9), Character::new(2));

        assert_eq!(
            tab.try_compare(eight_spaces),
            Err(IndentationError::InconsistentTabs)
        );
        assert_eq!(
            eight_spaces.try_compare(tab_and_space),
            Err(IndentationError::InconsistentTabs)
        );
        assert_eq!(tab.try_compare(tab_and_space), Ok(Ordering::Less));
    }
}
