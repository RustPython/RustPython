//! Datatypes to support source location information.

use std::fmt;

/// A location somewhere in the sourcecode.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Location {
    row: usize,
    column: usize,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "line {} column {}", self.row, self.column)
    }
}

impl Location {
    pub fn new(row: usize, column: usize) -> Self {
        Location { row, column }
    }

    pub fn row(&self) -> usize {
        self.row
    }

    pub fn column(&self) -> usize {
        self.column
    }

    pub fn reset(&mut self) {
        self.row = 1;
        self.column = 1;
    }

    pub fn go_right(&mut self) {
        self.column += 1;
    }

    pub fn newline(&mut self) {
        self.row += 1;
        self.column = 1;
    }
}
