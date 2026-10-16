//! Python code execution functions.

use crate::{
    Py, PyObjectRef, PyResult, VirtualMachine,
    builtins::PyCode,
    compiler::{self},
    scope::Scope,
};

impl VirtualMachine {
    /// PyRun_SimpleString
    ///
    /// Execute a string of Python code in a new scope with builtins.
    pub fn run_simple_string(&self, source: &str) -> PyResult {
        let scope = self.new_scope_with_builtins();
        self.run_string(scope, source, "<string>")
    }

    /// PyRun_String
    ///
    /// Execute a string of Python code with explicit scope and source path.
    pub fn run_string(&self, scope: Scope, source: &str, source_path: &str) -> PyResult {
        let code_obj = self
            .compile(source, compiler::Mode::Exec, source_path)
            .map_err(|err| err.into_pyexception(self, Some(source)))?;
        // linecache._register_code(code, source, filename)
        let _ = self.register_code_in_linecache(&code_obj, source);
        self.run_code_obj(code_obj, scope)
    }

    /// Register a code object's source in linecache._interactive_cache
    /// so that traceback can display source lines and caret indicators.
    pub fn register_code_in_linecache(&self, code: &Py<PyCode>, source: &str) -> PyResult<()> {
        let linecache = self.import("linecache", 0)?;
        let register = linecache.get_attr("_register_code", self)?;
        let source_str = self.ctx.new_str(source);
        let filename = self.ctx.new_str(code.source_path().as_str());
        register.call(
            (PyObjectRef::from(code.to_owned()), source_str, filename),
            self,
        )?;
        Ok(())
    }

    #[deprecated(note = "use run_string instead")]
    pub fn run_code_string(&self, scope: Scope, source: &str, source_path: &str) -> PyResult {
        self.run_string(scope, source, source_path)
    }

    pub fn run_block_expr(&self, scope: Scope, source: &str) -> PyResult {
        let code_obj = self
            .compile(source, compiler::Mode::BlockExpr, "<embedded>")
            .map_err(|err| err.into_pyexception(self, Some(source)))?;
        self.run_code_obj(code_obj, scope)
    }
}

#[cfg(feature = "host_env")]
mod file_run {
    use crate::{
        AsObject, Py, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyCode, PyDict},
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
            self.with_simple_run(path, |module_dict| {
                self.run_simple_file_inner(module_dict, scope, path)
            })
        }

        fn run_simple_file_inner(
            &self,
            module_dict: &Py<PyDict>,
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
                    .map_err(|_| self.new_runtime_error("Bad code object in .pyc file"))?;
                self.run_code_obj(code, scope)?;
            } else {
                if path != "<stdin>" {
                    set_main_loader(module_dict, path, "SourceFileLoader", self)?;
                }
                match crate::host_env::fs::read(path) {
                    Ok(source_bytes) => {
                        if let Some(null_at) = source_bytes.iter().position(|&b| b == 0) {
                            return Err(null_byte_syntax_error(self, path, &source_bytes, null_at));
                        }
                        #[cfg(feature = "parser")]
                        // Match compile() by honoring BOMs and encoding cookies in files.
                        let source = self.decode_source_bytes(&source_bytes, path, false)?;
                        #[cfg(not(feature = "parser"))]
                        let source = String::from_utf8(source_bytes)
                            .map_err(|err| self.new_os_error(err.to_string()))?;
                        let code_obj = self
                            .compile(&source, compiler::Mode::Exec, path)
                            .map_err(|err| err.into_pyexception(self, Some(&source)))?;
                        self.run_code_obj(code_obj, scope)?;
                    }
                    Err(err) => {
                        return Err(self.new_os_error(err.to_string()));
                    }
                }
            }
            Ok(())
        }

        // #[deprecated(note = "use rustpython::run_file instead; if this changes causes problems, please report an issue.")]
        pub fn run_script(&self, scope: Scope, path: &str) -> PyResult<()> {
            self.run_any_file(scope, path)
        }
    }

    fn null_byte_syntax_error(
        vm: &VirtualMachine,
        path: &str,
        source_bytes: &[u8],
        null_at: usize,
    ) -> PyBaseExceptionRef {
        let before = &source_bytes[..null_at];
        let lineno = before.iter().filter(|&&b| b == b'\n').count() + 1;
        let line_start = before
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |i| i + 1);
        let line = String::from_utf8_lossy(&source_bytes[line_start..null_at]);
        let syntax_error = vm.new_exception_msg(
            vm.ctx.exceptions.syntax_error.to_owned(),
            "source code cannot contain null bytes".into(),
        );
        let obj = syntax_error.as_object();
        obj.set_attr("filename", vm.ctx.new_str(path), vm).unwrap();
        obj.set_attr("lineno", vm.ctx.new_int(lineno), vm).unwrap();
        obj.set_attr("text", vm.ctx.new_str(format!("{line}\n")), vm)
            .unwrap();
        syntax_error
    }

    fn set_main_loader(
        module_dict: &Py<PyDict>,
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
        maybe_pyc_file_with_magic(path).unwrap_or(false)
    }

    fn maybe_pyc_file_with_magic(path: &str) -> std::io::Result<bool> {
        let path_obj = std::path::Path::new(path);
        if !path_obj.is_file() {
            return Ok(false);
        }

        let mut file = crate::host_env::fs::open(path)?;
        let mut buf = [0u8; 2];

        use std::io::{Read, Seek, SeekFrom};
        let n = file.read(&mut buf)?;
        // /dev/fd/N shares the file offset across every open of that path.
        // Restore it so a later read of the same script still starts at 0.
        let _ = file.seek(SeekFrom::Start(0));
        if n != 2 {
            return Ok(false);
        }

        // Read only two bytes of the magic. If the file was opened in
        // text mode, the bytes 3 and 4 of the magic (\r\n) might not
        // be read as they are on disk.
        Ok(crate::import::check_pyc_magic_number_bytes(&buf))
    }
}

/// A physical stdin line or an already decoded editor line.
pub enum InteractiveLine {
    Bytes(Vec<u8>),
    Text(String),
}

impl VirtualMachine {
    /// Resolve the current main module without rewriting an existing loader.
    pub fn stdin_main_module(&self) -> PyResult<crate::PyRef<crate::builtins::PyModule>> {
        let modules = self.sys_module.get_attr("modules", self)?;
        if let Ok(module) = modules.get_item("__main__", self)
            && let Ok(module) = module.downcast::<crate::builtins::PyModule>()
        {
            return Ok(module);
        }
        self.ensure_main_module()
    }

    /// Evaluate native interactive records. Input policy belongs to the caller;
    /// compilation, exception handling and namespace ownership stay in the VM.
    pub fn run_interactive(
        &self,
        mut readline: impl FnMut(&str) -> PyResult<Option<InteractiveLine>>,
    ) -> PyResult<()> {
        use crate::{AsObject, bytecode::CodeFlags};
        for (name, default) in [("ps1", ">>> "), ("ps2", "... ")] {
            if self.sys_module.dict().get_item_opt(name, self)?.is_none() {
                self.sys_module
                    .set_attr(name, self.ctx.new_str(default), self)?;
            }
        }
        let mut future_features = CodeFlags::empty();
        let mut memory_errors = 0;
        loop {
            let mut eof = false;
            let mut empty_eof = false;
            let result = (|| {
                let encoding = self
                    .sys_module
                    .get_attr("stdin", self)
                    .and_then(|stdin| stdin.get_attr("encoding", self))
                    .ok()
                    .and_then(|encoding| encoding.downcast::<crate::builtins::PyStr>().ok())
                    .and_then(|encoding| encoding.to_str().map(str::to_owned))
                    .unwrap_or_else(|| "utf-8".to_owned());
                let prompt = |name| {
                    self.sys_module
                        .get_attr(name, self)
                        .and_then(|prompt| prompt.str(self))
                        .ok()
                        .and_then(|prompt| prompt.to_str().map(str::to_owned))
                        .unwrap_or_default()
                };
                let ps1 = prompt("ps1");
                let ps2 = prompt("ps2");
                self.audit("compile", || (self.ctx.none(), self.ctx.new_str("<stdin>")))?;
                let mut source = String::new();
                let mut raw_source = Vec::new();
                #[cfg(feature = "parser")]
                let mut warnings = super::compile::InteractiveWarnings::default();
                loop {
                    let line = readline(if source.is_empty() { &ps1 } else { &ps2 })?;
                    let mut terminated = line.is_none();
                    let mut unterminated_fragment = false;
                    match line {
                        None => {
                            eof = true;
                            if source.is_empty() {
                                empty_eof = true;
                                return Ok(());
                            }
                            if !source.ends_with('\n') {
                                source.push('\n');
                            }
                        }
                        Some(InteractiveLine::Text(mut line)) => {
                            if !line.ends_with('\n') {
                                line.push('\n');
                            }
                            let line = line.replace("\r\n", "\n");
                            terminated = line == "\n";
                            source.push_str(&line);
                        }
                        Some(InteractiveLine::Bytes(line)) => {
                            unterminated_fragment = !line.ends_with(b"\n");
                            terminated = line == b"\n" || line == b"\r\n";
                            raw_source.extend_from_slice(&line);
                            source = self
                                .state
                                .codec_registry
                                .decode_text(
                                    self.ctx.new_bytes(raw_source.clone()).into(),
                                    &encoding,
                                    None,
                                    self,
                                )?
                                .to_str()
                                .ok_or_else(|| {
                                    self.new_exception_msg(
                                        self.ctx.exceptions.unicode_error.to_owned(),
                                        "stdin source contains surrogates".into(),
                                    )
                                })?
                                .replace("\r\n", "\n");
                        }
                    }
                    #[cfg(feature = "parser")]
                    self.emit_interactive_parser_warnings(
                        &source,
                        "<stdin>",
                        Some(identifier!(self, __main__)),
                        Some(compiler::parser::Mode::Module),
                        future_features.contains(CodeFlags::FUTURE_BARRY_AS_BDFL),
                        super::compile::ParserWarningState {
                            emitted_escapes: &[],
                            interactive: Some(&mut warnings),
                        },
                    )
                    .map_err(|error| {
                        super::compile::VmCompileError::Warning(error)
                            .into_pyexception(self, Some(&source))
                    })?;
                    let parsed = match compiler::interactive_input(
                        &source,
                        "<stdin>",
                        future_features,
                        terminated,
                    ) {
                        Ok(Some(parsed)) => parsed,
                        Ok(None) => continue,
                        Err(error) => {
                            let unclosed = matches!(&error, compiler::CompileError::Parse(error)
                                if error.is_unclosed_bracket || error.is_unclosed_string);
                            let error = super::compile::VmCompileError::Compile(error)
                                .into_pyexception_maybe_incomplete(
                                    self,
                                    Some(&source),
                                    !terminated,
                                );
                            if !eof
                                && (unclosed
                                    || (!terminated
                                        && error.fast_isinstance(
                                            self.ctx.exceptions.incomplete_input_error,
                                        )))
                            {
                                continue;
                            }
                            return Err(error);
                        }
                    };
                    // A raw final fragment is not a completed physical line.
                    // Warnings and definitive parser errors are observable now,
                    // but acceptance waits for the transport's EOF transaction.
                    if unterminated_fragment {
                        continue;
                    }
                    let main = self.stdin_main_module()?;
                    future_features |= parsed.future_features;
                    return (|| {
                        let filename = self.next_interactive_code_filename()?;
                        let code = self
                            .compile_after_parser_warnings(
                                &source,
                                if parsed.empty {
                                    compiler::Mode::Exec
                                } else {
                                    compiler::Mode::Single
                                },
                                filename,
                                compiler::CompileOpts {
                                    future_features,
                                    ..self.compile_opts()
                                },
                                Some(identifier!(self, __main__)),
                            )
                            .map_err(|error| error.into_pyexception(self, Some(&source)))?;
                        let linecache = self.import("linecache", 0)?;
                        linecache.get_attr("_register_code", self)?.call(
                            (
                                code.clone(),
                                self.ctx.new_str(source.as_str()),
                                self.ctx.new_str("<stdin>"),
                            ),
                            self,
                        )?;
                        self.audit("exec", || (code.clone(),))?;
                        self.run_code_obj(code, Scope::with_builtins(None, main.dict(), self))?;
                        Ok(())
                    })()
                    .inspect_err(|error| {
                        self.repair_interactive_syntax_error_text(error, &source)
                    });
                }
            })();
            if let Err(error) = result {
                if error.fast_isinstance(self.ctx.exceptions.memory_error) {
                    memory_errors += 1;
                    if memory_errors > 16 {
                        return Err(error);
                    }
                } else {
                    memory_errors = 0;
                }
                self.report_uncaught_exception(&error)?;
            } else {
                memory_errors = 0;
            }
            self.flush_io();
            if empty_eof {
                return Ok(());
            }
        }
    }

    fn repair_interactive_syntax_error_text(
        &self,
        error: &crate::builtins::PyBaseExceptionRef,
        source: &str,
    ) {
        use crate::AsObject;

        if !error.fast_isinstance(self.ctx.exceptions.syntax_error) {
            return;
        }
        // Match the interactive run_mod boundary: repair only text and keep
        // the original exception even if attribute access/conversion fails.
        let source = self.ctx.new_str(source);
        let lines = source.splitlines(crate::anystr::SplitLinesArgs { keepends: true }, self);
        let Ok(lineno) = error
            .as_object()
            .get_attr("lineno", self)
            .and_then(|lineno| lineno.try_into_value::<i32>(self))
        else {
            return;
        };
        if lineno > 0
            && let Some(line) = lines.get((lineno - 1) as usize)
        {
            let _ = error.as_object().set_attr("text", line.clone(), self);
        }
    }

    /// Native fd0 adapter used by sys._baserepl and non-terminal CLI inspection.
    pub fn run_basic_interactive(&self) -> PyResult<()> {
        self.run_interactive(|prompt| {
            let mut line = Vec::new();
            let mut prompt = prompt;
            loop {
                let result = crate::readline::stdio_readline_bytes(prompt, &mut line);
                prompt = "";
                match result {
                    Ok(0) => {
                        eprintln!();
                        return Ok(None);
                    }
                    Ok(_) => return Ok(Some(InteractiveLine::Bytes(line))),
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                        self.check_signals()?;
                    }
                    Err(_) if !line.is_empty() => return Ok(Some(InteractiveLine::Bytes(line))),
                    Err(_) => return Ok(None),
                }
            }
        })
    }

    /// Evaluate ordinary piped stdin as one file, without prompts or startup.
    pub fn run_stdin_file(&self) -> PyResult<()> {
        use std::io::Read;
        let main = self.stdin_main_module()?;
        self.with_simple_run("<stdin>", |_| {
            self.audit("compile", || (self.ctx.none(), self.ctx.new_str("<stdin>")))?;
            let mut bytes = Vec::new();
            loop {
                let result = std::io::stdin().lock().read_to_end(&mut bytes);
                match result {
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                        self.check_signals()?
                    }
                    _ => break,
                }
            }
            #[cfg(feature = "parser")]
            let source = self.decode_source_bytes(&bytes, "<stdin>", false)?;
            #[cfg(not(feature = "parser"))]
            let source =
                String::from_utf8(bytes).map_err(|error| self.new_os_error(error.to_string()))?;
            let code = self
                .compile_with_opts_and_module(
                    &source,
                    compiler::Mode::Exec,
                    "<stdin>",
                    self.compile_opts(),
                    Some(identifier!(self, __main__)),
                    &[],
                )
                .map_err(|error| error.into_pyexception(self, Some(&source)))?;
            self.audit("exec", || (code.clone(),))?;
            self.run_code_obj(code, Scope::with_builtins(None, main.dict(), self))
                .map(drop)
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::object::AsObject;
    use rustpython_vm::Interpreter;

    fn interpreter() -> Interpreter {
        Interpreter::without_stdlib(Default::default())
    }

    #[test]
    fn block_expr_return_const() {
        interpreter().enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            let value = vm.unwrap_pyresult(vm.run_block_expr(scope, "1"));
            let value = vm.unwrap_pyresult(value.try_int(vm));
            let value: u32 = vm.unwrap_pyresult(value.try_to_primitive(vm));
            assert_eq!(value, 1);
        })
    }

    #[test]
    fn block_expr_return_nonconst() {
        interpreter().enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            vm.unwrap_pyresult(scope.globals.set_item("x", vm.new_pyobj(3), vm));
            let value = vm.unwrap_pyresult(vm.run_block_expr(scope, "2 + x"));
            let value = vm.unwrap_pyresult(value.try_int(vm));
            let value: u32 = vm.unwrap_pyresult(value.try_to_primitive(vm));
            assert_eq!(value, 5);
        })
    }

    #[test]
    fn block_expr_return_function_def() {
        interpreter().enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            let value =
                vm.unwrap_pyresult(vm.run_block_expr(scope.clone(), "def f():\n    return 7"));
            vm.unwrap_pyresult(scope.globals.set_item("returned", value, vm));
            let value = vm.unwrap_pyresult(vm.run_block_expr(scope, "returned is f"));
            assert!(value.is(&vm.ctx.true_value));
        })
    }

    #[test]
    fn block_expr_return_class_def() {
        interpreter().enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            let value =
                vm.unwrap_pyresult(vm.run_block_expr(scope.clone(), "class C:\n    value = 11"));
            vm.unwrap_pyresult(scope.globals.set_item("returned", value, vm));
            let value = vm.unwrap_pyresult(vm.run_block_expr(scope, "returned is C"));
            assert!(value.is(&vm.ctx.true_value));
        })
    }
}
