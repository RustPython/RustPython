use crate::{
    AsObject, PyObjectRef, PyRef, PyResult, VirtualMachine,
    builtins::{PyCode, PyDictRef},
    compiler::{self, CompileError, CompileOpts},
    convert::TryFromObject,
    scope::Scope,
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

    // pymain_run_file_obj
    pub fn run_script(&self, scope: Scope, path: &str) -> PyResult<()> {
        // when pymain_run_module?
        if get_importer(path, self)?.is_some() {
            self.insert_sys_path(self.new_pyobj(path))?;
            let runpy = self.import("runpy", 0)?;
            let run_module_as_main = runpy.get_attr("_run_module_as_main", self)?;
            run_module_as_main.call((identifier!(self, __main__).to_owned(), false), self)?;
            return Ok(());
        }

        // TODO: check if this is proper place
        if !self.state.config.settings.safe_path {
            let dir = std::path::Path::new(path)
                .parent()
                .unwrap()
                .to_str()
                .unwrap();
            self.insert_sys_path(self.new_pyobj(dir))?;
        }

        self.run_any_file(scope, path)
    }

    // = _PyRun_AnyFileObject
    fn run_any_file(&self, scope: Scope, path: &str) -> PyResult<()> {
        let path = if path.is_empty() { "???" } else { path };

        self.run_simple_file(scope, path)
    }

    // = _PyRun_SimpleFileObject
    fn run_simple_file(&self, scope: Scope, path: &str) -> PyResult<()> {
        // __main__ is given by scope
        let sys_modules = self.sys_module.get_attr(identifier!(self, modules), self)?;
        let main_module = sys_modules.get_item(identifier!(self, __main__), self)?;
        let module_dict = main_module.dict().expect("main module must have __dict__");
        if !module_dict.contains_key(identifier!(self, __file__), self) {
            module_dict.set_item(
                identifier!(self, __file__),
                self.ctx.new_str(path).into(),
                self,
            )?;
            module_dict.set_item(identifier!(self, __cached__), self.ctx.none(), self)?;
        }

        // Consider to use enum to distinguish `path`
        // https://github.com/RustPython/RustPython/pull/6276#discussion_r2529849479

        // TODO: check .pyc here
        let pyc = false;
        if pyc {
            todo!("running pyc is not implemented yet");
        } else {
            if path != "<stdin>" {
                // TODO: set_main_loader(dict, filename, "SourceFileLoader");
            }
            // TODO: replace to something equivalent to py_run_file
            match std::fs::read_to_string(path) {
                Ok(source) => {
                    let code_obj = self
                        .compile(&source, compiler::Mode::Exec, path.to_owned())
                        .map_err(|err| self.new_syntax_error(&err, Some(&source)))?;
                    // trace!("Code object: {:?}", code_obj.borrow());
                    self.run_code_obj(code_obj, scope)?;
                }
                Err(err) => {
                    error!("Failed reading file '{path}': {err}");
                    // TODO: Need to change to ExitCode or Termination
                    std::process::exit(1);
                }
            }
        }
        Ok(())
    }

    // TODO: deprecate or reimplement using other primitive functions
    pub fn run_code_string(&self, scope: Scope, source: &str, source_path: String) -> PyResult {
        let code_obj = self
            .compile(source, compiler::Mode::Exec, source_path.clone())
            .map_err(|err| self.new_syntax_error(&err, Some(source)))?;
        // trace!("Code object: {:?}", code_obj.borrow());
        scope.globals.set_item(
            identifier!(self, __file__),
            self.new_pyobj(source_path),
            self,
        )?;
        self.run_code_obj(code_obj, scope)
    }

    pub fn run_block_expr(&self, scope: Scope, source: &str) -> PyResult {
        let code_obj = self
            .compile(source, compiler::Mode::BlockExpr, "<embedded>".to_owned())
            .map_err(|err| self.new_syntax_error(&err, Some(source)))?;
        // trace!("Code object: {:?}", code_obj.borrow());
        self.run_code_obj(code_obj, scope)
    }
}

fn get_importer(path: &str, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
    let path_importer_cache = vm.sys_module.get_attr("path_importer_cache", vm)?;
    let path_importer_cache = PyDictRef::try_from_object(vm, path_importer_cache)?;
    if let Some(importer) = path_importer_cache.get_item_opt(path, vm)? {
        return Ok(Some(importer));
    }
    let path = vm.ctx.new_str(path);
    let path_hooks = vm.sys_module.get_attr("path_hooks", vm)?;
    let mut importer = None;
    let path_hooks: Vec<PyObjectRef> = path_hooks.try_into_value(vm)?;
    for path_hook in path_hooks {
        match path_hook.call((path.clone(),), vm) {
            Ok(imp) => {
                importer = Some(imp);
                break;
            }
            Err(e) if e.fast_isinstance(vm.ctx.exceptions.import_error) => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(if let Some(imp) = importer {
        let imp = path_importer_cache.get_or_insert(vm, path.into(), || imp.clone())?;
        Some(imp)
    } else {
        None
    })
}
