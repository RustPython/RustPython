use crate::{
    builtins::PyCode,
    compile::{self, CompileError, CompileOpts},
    PyRef, VirtualMachine,
};

impl VirtualMachine {
    /// Returns a basic CompileOpts instance with options accurate to the vm. Used
    /// as the CompileOpts for `vm.compile()`.
    pub fn compile_opts(&self) -> CompileOpts {
        CompileOpts {
            optimize: self.state.settings.optimize,
        }
    }

    pub fn compile(
        &self,
        source: &str,
        mode: compile::Mode,
        source_path: String,
    ) -> Result<PyRef<PyCode>, CompileError> {
        self.compile_with_opts(source, mode, source_path, self.compile_opts())
    }

    pub fn compile_with_opts(
        &self,
        source: &str,
        mode: compile::Mode,
        source_path: String,
        opts: CompileOpts,
    ) -> Result<PyRef<PyCode>, CompileError> {
        compile::compile(source, mode, source_path, opts).map(|code| self.ctx.new_code(code))
    }
}
