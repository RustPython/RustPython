/// A Python source range with signed line and column coordinates.
///
/// Unlike a parser text range, this preserves absent coordinates (negative
/// values), as well as line zero used by synthetic RESUME instructions.
/// Columns are zero-based; positive line numbers are one-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceLocationRange {
    pub line: i32,
    pub end_line: i32,
    pub col: i32,
    pub end_col: i32,
}

/// Location metadata shared by AST conversion and code generation.
///
/// Numeric IDs occupy Ruff's top-level node identity range. Keeping the store
/// independent of Ruff allows AST-only VM builds without the codegen crate.
#[derive(Default, Debug)]
pub struct AstLocations {
    locations: alloc::collections::BTreeMap<u32, SourceLocationRange>,
    next_index: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AstLocationError {
    ConflictingLocation,
    ExhaustedNodeIdentities,
    CoordinateOverflow,
}

impl AstLocations {
    // Ruff reserves the upper two bits for sub-AST levels and sentinel IDs.
    pub const MAX_ID: u32 = (1 << 30) - 1;

    pub fn allocate(
        &mut self,
        location: Option<SourceLocationRange>,
    ) -> Result<u32, AstLocationError> {
        if self.next_index > Self::MAX_ID {
            return Err(AstLocationError::ExhaustedNodeIdentities);
        }
        let id = self.next_index;
        self.next_index = self
            .next_index
            .checked_add(1)
            .ok_or(AstLocationError::ExhaustedNodeIdentities)?;
        if let Some(location) = location {
            self.insert(id, location)?;
        }
        Ok(id)
    }

    /// An inherited identity may be shared only with identical coordinates.
    pub fn insert(
        &mut self,
        id: u32,
        location: SourceLocationRange,
    ) -> Result<(), AstLocationError> {
        if id > Self::MAX_ID {
            return Err(AstLocationError::ExhaustedNodeIdentities);
        }
        if let Some(previous) = self.locations.get(&id) {
            return if *previous == location {
                Ok(())
            } else {
                Err(AstLocationError::ConflictingLocation)
            };
        }
        self.locations.insert(id, location);
        self.next_index = self.next_index.max(id + 1);
        Ok(())
    }

    pub fn get(&self, id: u32) -> Option<SourceLocationRange> {
        self.locations.get(&id).copied()
    }
}
