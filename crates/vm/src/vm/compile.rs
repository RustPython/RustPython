//! Python code compilation functions.
//!
//! For code execution functions, see python_run.rs

use crate::{
    PyRef, VirtualMachine,
    builtins::PyCode,
    compiler::{self, CompileError, CompileOpts},
};

impl VirtualMachine {
    pub fn compile(
        &self,
        source: &str,
        mode: compiler::Mode,
        source_path: String,
    ) -> Result<PyRef<PyCode>, CompileError> {
        self.compile_with_opts(source, mode, source_path, self.compile_opts())
    }

    pub fn compile_with_opts(
        &self,
        source: &str,
        mode: compiler::Mode,
        source_path: String,
        opts: CompileOpts,
    ) -> Result<PyRef<PyCode>, CompileError> {
        compiler::compile(source, mode, &source_path, opts).map(|code| self.ctx.new_code(code))
    }
}
