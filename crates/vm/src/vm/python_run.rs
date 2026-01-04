//! Python code execution functions.

use crate::{
    PyResult, VirtualMachine,
    builtins::{PyCode, PyDictRef},
    compiler::{self},
    scope::Scope,
};

impl VirtualMachine {
    /// _PyRun_AnyFileObject (internal)
    ///
    /// Execute a Python file. Currently always delegates to run_simple_file
    /// (interactive mode is handled separately in shell.rs).
    ///
    /// Note: This is an internal function. Use `run_file` for the public interface.
    #[doc(hidden)]
    pub fn run_any_file(&self, scope: Scope, path: &str) -> PyResult<()> {
        let path = if path.is_empty() { "???" } else { path };
        self.run_simple_file(scope, path)
    }

    /// _PyRun_SimpleFileObject
    ///
    /// Execute a Python file with __main__ module setup.
    /// Sets __file__ and __cached__ before execution, removes them after.
    fn run_simple_file(&self, scope: Scope, path: &str) -> PyResult<()> {
        let sys_modules = self.sys_module.get_attr(identifier!(self, modules), self)?;
        let main_module = sys_modules.get_item(identifier!(self, __main__), self)?;
        let module_dict = main_module.dict().expect("main module must have __dict__");

        // Track whether we set __file__ (for cleanup)
        let set_file_name = !module_dict.contains_key(identifier!(self, __file__), self);
        if set_file_name {
            module_dict.set_item(
                identifier!(self, __file__),
                self.ctx.new_str(path).into(),
                self,
            )?;
            module_dict.set_item(identifier!(self, __cached__), self.ctx.none(), self)?;
        }

        let result = self.run_simple_file_inner(&module_dict, scope, path);

        self.flush_io();

        // Cleanup __file__ and __cached__ after execution
        if set_file_name {
            let _ = module_dict.del_item(identifier!(self, __file__), self);
            let _ = module_dict.del_item(identifier!(self, __cached__), self);
        }

        result
    }

    fn run_simple_file_inner(
        &self,
        module_dict: &PyDictRef,
        scope: Scope,
        path: &str,
    ) -> PyResult<()> {
        let pyc = maybe_pyc_file(path);
        if pyc {
            // pyc file execution
            set_main_loader(module_dict, path, "SourcelessFileLoader", self)?;
            let loader = module_dict.get_item("__loader__", self)?;
            let get_code = loader.get_attr("get_code", self)?;
            let code_obj = get_code.call((identifier!(self, __main__).to_owned(),), self)?;
            let code = code_obj
                .downcast::<PyCode>()
                .map_err(|_| self.new_runtime_error("Bad code object in .pyc file".to_owned()))?;
            self.run_code_obj(code, scope)?;
        } else {
            if path != "<stdin>" {
                set_main_loader(module_dict, path, "SourceFileLoader", self)?;
            }
            match std::fs::read_to_string(path) {
                Ok(source) => {
                    let code_obj = self
                        .compile(&source, compiler::Mode::Exec, path.to_owned())
                        .map_err(|err| self.new_syntax_error(&err, Some(&source)))?;
                    self.run_code_obj(code_obj, scope)?;
                }
                Err(err) => {
                    return Err(self.new_os_error(err.to_string()));
                }
            }
        }
        Ok(())
    }

    /// PyRun_SimpleString
    ///
    /// Execute a string of Python code in a new scope with builtins.
    pub fn run_simple_string(&self, source: &str) -> PyResult {
        let scope = self.new_scope_with_builtins();
        self.run_string(scope, source, "<string>".to_owned())
    }

    /// PyRun_String
    ///
    /// Execute a string of Python code with explicit scope and source path.
    pub fn run_string(&self, scope: Scope, source: &str, source_path: String) -> PyResult {
        let code_obj = self
            .compile(source, compiler::Mode::Exec, source_path)
            .map_err(|err| self.new_syntax_error(&err, Some(source)))?;
        self.run_code_obj(code_obj, scope)
    }

    #[deprecated(note = "use run_string instead")]
    pub fn run_code_string(&self, scope: Scope, source: &str, source_path: String) -> PyResult {
        self.run_string(scope, source, source_path)
    }

    // #[deprecated(note = "use rustpython::run_file instead; if this changes causes problems, please report an issue.")]
    pub fn run_script(&self, scope: Scope, path: &str) -> PyResult<()> {
        self.run_any_file(scope, path)
    }

    pub fn run_block_expr(&self, scope: Scope, source: &str) -> PyResult {
        let code_obj = self
            .compile(source, compiler::Mode::BlockExpr, "<embedded>".to_owned())
            .map_err(|err| self.new_syntax_error(&err, Some(source)))?;
        self.run_code_obj(code_obj, scope)
    }

    /// flush_io
    ///
    /// Flush stdout and stderr. Errors are silently ignored.
    fn flush_io(&self) {
        if let Ok(stdout) = self.sys_module.get_attr("stdout", self) {
            let _ = self.call_method(&stdout, identifier!(self, flush).as_str(), ());
        }
        if let Ok(stderr) = self.sys_module.get_attr("stderr", self) {
            let _ = self.call_method(&stderr, identifier!(self, flush).as_str(), ());
        }
    }
}

fn set_main_loader(
    module_dict: &PyDictRef,
    filename: &str,
    loader_name: &str,
    vm: &VirtualMachine,
) -> PyResult<()> {
    vm.import("importlib.machinery", 0)?;
    let sys_modules = vm.sys_module.get_attr(identifier!(vm, modules), vm)?;
    let machinery = sys_modules.get_item("importlib.machinery", vm)?;
    let loader_name = vm.ctx.new_str(loader_name);
    let loader_class = machinery.get_attr(&loader_name, vm)?;
    let loader = loader_class.call((identifier!(vm, __main__).to_owned(), filename), vm)?;
    module_dict.set_item("__loader__", loader, vm)?;
    Ok(())
}

/// Check whether a file is maybe a pyc file.
///
/// Detection is performed by:
/// 1. Checking if the filename ends with ".pyc"
/// 2. If not, reading the first 2 bytes and comparing with the magic number
fn maybe_pyc_file(path: &str) -> bool {
    if path.ends_with(".pyc") {
        return true;
    }
    maybe_pyc_file_with_magic(path, &crate::version::PYC_MAGIC_NUMBER_BYTES).unwrap_or(false)
}

fn maybe_pyc_file_with_magic(path: &str, magic_number: &[u8]) -> std::io::Result<bool> {
    let path_obj = std::path::Path::new(path);
    if !path_obj.is_file() {
        return Ok(false);
    }

    let mut file = std::fs::File::open(path)?;
    let mut buf = [0u8; 2];

    use std::io::Read;
    if file.read(&mut buf)? != 2 || magic_number.len() < 2 {
        return Ok(false);
    }

    // Read only two bytes of the magic. If the file was opened in
    // text mode, the bytes 3 and 4 of the magic (\r\n) might not
    // be read as they are on disk.
    Ok(buf == magic_number[..2])
}
