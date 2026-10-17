//! Python code execution functions.

use crate::{
    AsObject, Py, PyObjectRef, PyResult, VirtualMachine,
    builtins::{PyBaseException, PyBaseExceptionRef, PyCode, PyModule, PyStr},
    bytecode::CodeFlags,
    compiler::{self},
    scope::Scope,
    stdlib,
};
use core::sync::atomic::Ordering;

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
        Py, PyResult, VirtualMachine,
        builtins::{PyCode, PyDict},
        compiler::{self},
        scope::Scope,
    };

    impl VirtualMachine {
        /// _PyRun_AnyFileObject (internal)
        ///
        /// Execute a Python file. A named file is not interactive, so this
        /// always delegates to run_simple_file; see `run_any_stdin` for stdin.
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
                            return Err(super::null_byte_syntax_error(
                                self,
                                path,
                                &source_bytes,
                                null_at,
                            ));
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

/// One line of interactive input: raw bytes read from stdin, or text from a
/// line editor.
pub enum InteractiveLine {
    Bytes(Vec<u8>),
    Text(String),
}

/// Source of one interactive statement, ready to be compiled.
struct InteractiveSource {
    source: String,
    parsed: compiler::InteractiveInput,
}

impl VirtualMachine {
    /// _Py_FdIsInteractive(stdin, "<stdin>")
    pub fn stdin_is_interactive(&self) -> bool {
        use std::io::IsTerminal;
        std::io::stdin().is_terminal() || self.state.config.settings.interactive
    }

    /// _PyRun_AnyFile(stdin, "<stdin>")
    ///
    /// Run stdin as an interactive session or as a script.
    pub fn run_any_stdin(&self) -> PyResult<()> {
        if self.stdin_is_interactive() {
            self.run_interactive_loop(|prompt| self.stdio_readline(prompt))
        } else {
            self.run_simple_stdin()
        }
    }

    /// _PyRun_InteractiveLoop
    ///
    /// `readline` plays the role of `PyOS_Readline`: it returns `Ok(None)` at
    /// the end of input.
    pub fn run_interactive_loop(
        &self,
        mut readline: impl FnMut(&str) -> PyResult<Option<InteractiveLine>>,
    ) -> PyResult<()> {
        for (name, default) in [("ps1", ">>> "), ("ps2", "... ")] {
            if self.sys_module.dict().get_item_opt(name, self)?.is_none() {
                let _ = self
                    .sys_module
                    .set_attr(name, self.ctx.new_str(default), self);
            }
        }
        let mut flags = CodeFlags::empty();
        let mut nomem_count = 0;
        loop {
            match self.run_interactive_one(&mut readline, &mut flags) {
                Ok(true) => nomem_count = 0,
                // E_EOF
                Ok(false) => return Ok(()),
                Err(exc) => {
                    // Prevent an endless loop after multiple consecutive
                    // MemoryErrors while still allowing an interactive
                    // command to fail with a MemoryError.
                    if exc.fast_isinstance(self.ctx.exceptions.memory_error) {
                        nomem_count += 1;
                        if nomem_count > 16 {
                            return Err(exc);
                        }
                    } else {
                        nomem_count = 0;
                    }
                    self.print_uncaught_exception(&exc);
                    self.flush_io();
                }
            }
        }
    }

    /// _PyRun_InteractiveOne
    ///
    /// Read, compile and run one interactive statement in `__main__`.
    /// Return `Ok(false)` at the end of input.
    fn run_interactive_one(
        &self,
        readline: &mut impl FnMut(&str) -> PyResult<Option<InteractiveLine>>,
        flags: &mut CodeFlags,
    ) -> PyResult<bool> {
        let Some(InteractiveSource { source, parsed }) =
            self.interactive_parse(readline, *flags)?
        else {
            return Ok(false);
        };
        let main_module = self.add_main_module()?;
        // _PyAST_Compile merges the future features of the parsed statement
        // into the flags before code generation.
        *flags |= parsed.future_features;
        self.run_interactive_mod(&source, parsed.empty, *flags, &main_module)
            .inspect_err(|exc| self.fix_syntax_error_text(exc, &source))?;
        self.flush_io();
        Ok(true)
    }

    /// pyrun_one_parse_ast
    ///
    /// Read lines with `sys.ps1` and `sys.ps2` prompts until they form one
    /// complete interactive statement. Return `None` at the end of input.
    fn interactive_parse(
        &self,
        readline: &mut impl FnMut(&str) -> PyResult<Option<InteractiveLine>>,
        flags: CodeFlags,
    ) -> PyResult<Option<InteractiveSource>> {
        let encoding = self
            .sys_module
            .get_attr("stdin", self)
            .and_then(|stdin| stdin.get_attr("encoding", self))
            .ok()
            .and_then(|encoding| encoding.downcast::<PyStr>().ok())
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

        // _PyParser_InteractiveASTFromFile
        self.audit("compile", || (self.ctx.none(), self.ctx.new_str("<stdin>")))?;
        let mut source = String::new();
        let mut raw_source = Vec::new();
        #[cfg(feature = "parser")]
        let mut warnings = super::compile::InteractiveWarnings::default();
        loop {
            let line = readline(if source.is_empty() { &ps1 } else { &ps2 });
            // tok_underflow_interactive ends the prompt line at EOF or on an
            // interrupted read.
            if !matches!(line, Ok(Some(_))) {
                writeln!(stdlib::sys::PyStderr(self));
            }
            let (terminated, unterminated_fragment, eof) = match line? {
                None if source.is_empty() => return Ok(None),
                None => {
                    if !source.ends_with('\n') {
                        source.push('\n');
                    }
                    (true, false, true)
                }
                Some(InteractiveLine::Text(mut line)) => {
                    if !line.ends_with('\n') {
                        line.push('\n');
                    }
                    let line = line.replace("\r\n", "\n");
                    let terminated = line == "\n";
                    source.push_str(&line);
                    (terminated, false, false)
                }
                Some(InteractiveLine::Bytes(line)) => {
                    let unterminated_fragment = !line.ends_with(b"\n");
                    let terminated = line == b"\n" || line == b"\r\n";
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
                    (terminated, unterminated_fragment, false)
                }
            };
            #[cfg(feature = "parser")]
            self.emit_interactive_parser_warnings(
                &source,
                "<stdin>",
                Some(identifier!(self, __main__)),
                flags.contains(CodeFlags::FUTURE_BARRY_AS_BDFL),
                &mut warnings,
            )
            .map_err(|error| {
                super::compile::VmCompileError::Warning(error).into_pyexception(self, Some(&source))
            })?;
            let parsed = match compiler::interactive_input(&source, "<stdin>", flags, terminated) {
                Ok(Some(parsed)) => parsed,
                Ok(None) => continue,
                Err(error) => {
                    let unclosed = matches!(&error, compiler::CompileError::Parse(error)
                        if error.is_unclosed_bracket || error.is_unclosed_string);
                    let error = super::compile::VmCompileError::Compile(error)
                        .into_pyexception_maybe_incomplete(self, Some(&source), !terminated);
                    if !eof
                        && (unclosed
                            || (!terminated
                                && error
                                    .fast_isinstance(self.ctx.exceptions.incomplete_input_error)))
                    {
                        continue;
                    }
                    return Err(error);
                }
            };
            // A final fragment without a newline is not a complete line yet.
            // Its warnings and definitive syntax errors are reported now, but
            // it is only accepted once the next read reports EOF.
            if unterminated_fragment {
                continue;
            }
            return Ok(Some(InteractiveSource { source, parsed }));
        }
    }

    /// run_mod for interactive input
    fn run_interactive_mod(
        &self,
        source: &str,
        empty: bool,
        flags: CodeFlags,
        main_module: &Py<PyModule>,
    ) -> PyResult<()> {
        let filename = get_interactive_filename(
            "<stdin>",
            self.state
                .interactive_src_count
                .fetch_add(1, Ordering::Relaxed),
        );
        let mode = if empty {
            compiler::Mode::Exec
        } else {
            compiler::Mode::Single
        };
        let opts = compiler::CompileOpts {
            future_features: flags,
            ..self.compile_opts()
        };
        let code = self
            .compile_after_parser_warnings(
                source,
                mode,
                filename,
                opts,
                Some(identifier!(self, __main__)),
            )
            .map_err(|error| error.into_pyexception(self, Some(source)))?;
        let linecache = self.import("linecache", 0)?;
        linecache.get_attr("_register_code", self)?.call(
            (
                code.clone(),
                self.ctx.new_str(source),
                self.ctx.new_str("<stdin>"),
            ),
            self,
        )?;
        self.audit("exec", || (code.clone(),))?;
        let globals = main_module.dict();
        self.run_code_obj(code, Scope::with_builtins(None, globals, self))?;
        Ok(())
    }

    /// Fix the `text` attribute of a SyntaxError raised by interactive input.
    fn fix_syntax_error_text(&self, exc: &Py<PyBaseException>, source: &str) {
        if !exc.fast_isinstance(self.ctx.exceptions.syntax_error) {
            return;
        }
        let source = self.ctx.new_str(source);
        let lines = source.splitlines(crate::anystr::SplitLinesArgs { keepends: true }, self);
        let Ok(lineno) = exc
            .as_object()
            .get_attr("lineno", self)
            .and_then(|lineno| lineno.try_into_value::<i32>(self))
        else {
            return;
        };
        if lineno > 0
            && let Some(line) = lines.get((lineno - 1) as usize)
        {
            let _ = exc.as_object().set_attr("text", line.clone(), self);
        }
    }

    /// PyOS_StdioReadline
    ///
    /// Write the prompt to stderr and read one line from stdin.
    fn stdio_readline(&self, prompt: &str) -> PyResult<Option<InteractiveLine>> {
        let mut line = Vec::new();
        let mut prompt = prompt;
        loop {
            let result = crate::readline::stdio_readline_bytes(prompt, &mut line);
            prompt = "";
            match result {
                Ok(0) if line.is_empty() => return Ok(None),
                Ok(_) => return Ok(Some(InteractiveLine::Bytes(line))),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                    self.check_signals()?;
                }
                // A read error ends the input like EOF.
                Err(_) if line.is_empty() => return Ok(None),
                Err(_) => return Ok(Some(InteractiveLine::Bytes(line))),
            }
        }
    }

    /// _PyRun_SimpleFile(stdin, "<stdin>")
    fn run_simple_stdin(&self) -> PyResult<()> {
        use std::io::Read;
        self.with_simple_run("<stdin>", |module_dict| {
            // _PyParser_ASTFromFile
            self.audit("compile", || (self.ctx.none(), self.ctx.new_str("<stdin>")))?;
            let mut bytes = Vec::new();
            let mut buf = [0; 8192];
            loop {
                match std::io::stdin().read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => bytes.extend_from_slice(&buf[..n]),
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                        self.check_signals()?;
                    }
                    // A read error ends the input like EOF.
                    Err(_) => break,
                }
            }
            if let Some(null_at) = bytes.iter().position(|&b| b == 0) {
                return Err(null_byte_syntax_error(self, "<stdin>", &bytes, null_at));
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
            let scope = Scope::with_builtins(None, module_dict.to_owned(), self);
            self.run_code_obj(code, scope).map(drop)
        })
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

/// get_interactive_filename
fn get_interactive_filename(filename: &str, count: usize) -> String {
    match filename
        .strip_prefix('<')
        .and_then(|name| name.strip_suffix('>'))
    {
        Some(middle) => format!("<{middle}-{count}>"),
        None => format!("{filename}-{count}"),
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
