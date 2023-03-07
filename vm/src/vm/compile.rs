use crate::{
    builtins::{PyCode, PyDictRef},
    compiler::{self, CompileError, CompileOpts},
    convert::TryFromObject,
    scope::Scope,
    AsObject, PyObjectRef, PyRef, PyResult, VirtualMachine,
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
        compiler::compile(source, mode, source_path, opts).map(|code| self.ctx.new_code(code))
    }

    pub fn run_script(&self, scope: Scope, path: &str) -> PyResult<()> {
        if get_importer(path, self)?.is_some() {
            self.insert_sys_path(self.new_pyobj(path))?;
            let runpy = self.import("runpy", None, 0)?;
            let run_module_as_main = runpy.get_attr("_run_module_as_main", self)?;
            run_module_as_main.call((identifier!(self, __main__).to_owned(), false), self)?;
            return Ok(());
        }

        let dir = std::path::Path::new(path)
            .parent()
            .unwrap()
            .to_str()
            .unwrap();
        self.insert_sys_path(self.new_pyobj(dir))?;

        match std::fs::read_to_string(path) {
            Ok(source) => {
                self.run_code_string(scope, &source, path.to_owned())?;
            }
            Err(err) => {
                error!("Failed reading file '{}': {}", path, err);
                // TODO: Need to change to ExitCode or Termination
                std::process::exit(1);
            }
        }
        Ok(())
    }

    pub fn run_code_string(&self, scope: Scope, source: &str, source_path: String) -> PyResult {
        let code_obj = self
            .compile(source, compiler::Mode::Exec, source_path.clone())
            .map_err(|err| self.new_syntax_error(&err))?;
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
            .map_err(|err| self.new_syntax_error(&err))?;
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
