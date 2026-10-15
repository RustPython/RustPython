//! Per-conversion coordinates for Python AST input and output.

use crate::{PyResult, VirtualMachine};
use core::{cell::RefCell, ops::Deref};
use ruff_python_ast::{AtomicNodeIndex, NodeIndex};
use ruff_text_size::TextRange;
use rustpython_compiler_core::{AstLocations, SourceFile, SourceLocationRange};

pub(super) struct AstSourceContext {
    source_file: SourceFile,
    // Outbound parser AST conversion never allocates a metadata table.
    locations: RefCell<Option<AstLocations>>,
}

#[derive(Clone, Copy)]
pub(super) struct AstNodeRange {
    pub range: TextRange,
    pub index: NodeIndex,
}

impl AstNodeRange {
    pub fn node_index(self) -> AtomicNodeIndex {
        let index = AtomicNodeIndex::NONE;
        index.set(self.index);
        index
    }
}

impl AstSourceContext {
    pub fn new(source_file: SourceFile) -> Self {
        Self {
            source_file,
            locations: RefCell::new(None),
        }
    }

    /// Coordinates have already been read and validated by the caller. The
    /// metadata borrow cannot span Python getters or recursive node conversion.
    pub fn register(
        &self,
        vm: &VirtualMachine,
        range: TextRange,
        location: Option<SourceLocationRange>,
    ) -> PyResult<AstNodeRange> {
        let id = self
            .locations
            .borrow_mut()
            .get_or_insert_default()
            .allocate(location)
            .map_err(|_| vm.new_memory_error("too many AST location identities"))?;
        Ok(AstNodeRange {
            range,
            index: NodeIndex::from(id),
        })
    }

    pub fn location(&self, index: NodeIndex) -> Option<SourceLocationRange> {
        let id = index.as_u32()?;
        self.locations.borrow().as_ref()?.get(id)
    }

    #[cfg(feature = "codegen")]
    pub fn preprocess_locations(
        &self,
        f: impl FnOnce(&mut AstLocations) -> Result<(), rustpython_compiler_core::AstLocationError>,
    ) -> Result<(), rustpython_compiler_core::AstLocationError> {
        f(self.locations.borrow_mut().get_or_insert_default())
    }

    pub fn has_locations(&self) -> bool {
        self.locations.borrow().is_some()
    }

    pub fn take_locations(&self) -> Option<AstLocations> {
        self.locations.borrow_mut().take()
    }
}

impl Deref for AstSourceContext {
    type Target = SourceFile;

    fn deref(&self) -> &Self::Target {
        &self.source_file
    }
}

impl Deref for AstNodeRange {
    type Target = TextRange;

    fn deref(&self) -> &Self::Target {
        &self.range
    }
}
