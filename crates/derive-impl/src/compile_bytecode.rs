//! Parsing and processing for this form:
//! ```ignore
//! py_compile!(
//!     // either:
//!     source = "python_source_code",
//!     // or
//!     file = "file/path/relative/to/this/file",
//!
//!     // the mode to compile the code in
//!     mode = "exec", // or "eval" or "single"
//!     // the path put into the CodeObject, defaults to "frozen"
//!     module_name = "frozen",
//! )
//! ```

use crate::Diagnostic;
use alloc::collections::BTreeMap;
use proc_macro2::{Span, TokenStream};
use quote::quote;
use rustpython_compiler_core::{Mode, bytecode::CodeObject, frozen};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
use syn::{
    self, LitByteStr, LitStr, Macro,
    parse::{ParseStream, Parser, Result as ParseResult},
    spanned::Spanned,
};

enum CompilationSourceKind {
    /// Source is a File (Path)
    File { base: PathBuf, rel_path: PathBuf },
    /// Direct Raw source code
    SourceCode(String),
    /// Source is a directory
    Dir { base: PathBuf, rel_path: PathBuf },
}

struct CompiledModule {
    code: CodeObject,
    package: bool,
    namespace_container: bool,
}

struct CompilationSource {
    kind: CompilationSourceKind,
    span: (Span, Span),
}

/// Read a source file as text. A byte order mark belongs to the file's
/// encoding rather than to the text, so it never reaches the compiler.
fn read_source_file(path: &Path) -> std::io::Result<String> {
    let mut source = fs::read_to_string(path)?;
    if source.starts_with('\u{feff}') {
        source.drain(..'\u{feff}'.len_utf8());
    }
    Ok(source)
}

pub trait Compiler {
    fn compile(
        &self,
        source: &str,
        mode: Mode,
        module_name: &str,
    ) -> Result<CodeObject, Box<dyn core::error::Error>>;
}

impl CompilationSource {
    fn compile_string<D: core::fmt::Display, F: FnOnce() -> D>(
        &self,
        source: &str,
        mode: Mode,
        module_name: &str,
        compiler: &dyn Compiler,
        origin: F,
    ) -> Result<CodeObject, Diagnostic> {
        compiler.compile(source, mode, module_name).map_err(|err| {
            Diagnostic::spans_error(
                self.span,
                format!("Python compile error from {}: {}", origin(), err),
            )
        })
    }

    fn compile(
        &self,
        mode: Mode,
        module_name: &str,
        compiler: &dyn Compiler,
    ) -> Result<HashMap<String, CompiledModule>, Diagnostic> {
        match &self.kind {
            CompilationSourceKind::Dir { base, rel_path } => {
                let mut containers = BTreeMap::new();
                let mut modules = self.compile_dir(
                    base,
                    &base.join(rel_path),
                    "",
                    mode,
                    compiler,
                    &mut containers,
                )?;
                // Wait until all genuine modules are known: foo.py must beat a
                // generated foo/ container regardless of directory iteration order.
                for (name, path) in containers {
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        modules.entry(name.clone())
                    {
                        let code = self
                            .compile_string("", Mode::Exec, &name, compiler, || path.display())?;
                        entry.insert(CompiledModule {
                            code,
                            package: true,
                            namespace_container: true,
                        });
                    }
                }
                Ok(modules)
            }
            _ => Ok(core::iter::once((
                module_name.to_string(),
                CompiledModule {
                    code: self.compile_single(mode, module_name, compiler)?,
                    package: false,
                    namespace_container: false,
                },
            ))
            .collect()),
        }
    }

    fn compile_single(
        &self,
        mode: Mode,
        module_name: &str,
        compiler: &dyn Compiler,
    ) -> Result<CodeObject, Diagnostic> {
        match &self.kind {
            CompilationSourceKind::File { base, rel_path } => {
                let path = base.join(rel_path);
                let source = read_source_file(&path).map_err(|err| {
                    Diagnostic::spans_error(
                        self.span,
                        format!("Error reading file {path:?}: {err}"),
                    )
                })?;
                self.compile_string(&source, mode, module_name, compiler, || rel_path.display())
            }
            CompilationSourceKind::SourceCode(code) => self.compile_string(
                &textwrap::dedent(code),
                mode,
                module_name,
                compiler,
                || "string literal",
            ),
            CompilationSourceKind::Dir { .. } => {
                unreachable!("Can't use compile_single with directory source")
            }
        }
    }

    fn compile_dir(
        &self,
        base: &Path,
        path: &Path,
        parent: &str,
        mode: Mode,
        compiler: &dyn Compiler,
        containers: &mut BTreeMap<String, PathBuf>,
    ) -> Result<HashMap<String, CompiledModule>, Diagnostic> {
        let mut has_initializer = false;
        let mut code_map = HashMap::new();
        let paths = fs::read_dir(path)
            .or_else(|e| {
                // Without symlink support git checks a symlink out as a text file
                // holding its target, which is relative to the file's own directory.
                if cfg!(windows)
                    && let Ok(target) = fs::read_to_string(path)
                {
                    let target_dir = path
                        .parent()
                        .unwrap_or_else(|| Path::new(""))
                        .join(target.trim());
                    return fs::read_dir(target_dir);
                }
                Err(e)
            })
            .map_err(|err| {
                Diagnostic::spans_error(self.span, format!("Error listing dir {path:?}: {err}"))
            })?;
        for path in paths {
            let path = path.map_err(|err| {
                Diagnostic::spans_error(self.span, format!("Failed to list file: {err}"))
            })?;
            let path = path.path();
            let file_name = path.file_name().unwrap().to_str().ok_or_else(|| {
                Diagnostic::spans_error(self.span, format!("Invalid UTF-8 in file name {path:?}"))
            })?;
            if path.is_dir() {
                let nparent = if parent.is_empty() {
                    file_name.to_string()
                } else {
                    format!("{parent}.{file_name}")
                };
                code_map
                    .extend(self.compile_dir(base, &path, &nparent, mode, compiler, containers)?);
            } else if file_name.ends_with(".py") {
                has_initializer |= file_name == "__init__.py";
                // These fixtures must be parsed at import time to exercise syntax errors.
                if parent == "test.test_lazy_import.data.badsyntax" {
                    continue;
                }
                let stem = path.file_stem().unwrap().to_str().unwrap();
                let is_init = stem == "__init__";
                let module_name = if is_init {
                    parent
                } else if parent.is_empty() {
                    stem
                } else {
                    &format!("{parent}.{stem}")
                };

                let compile_path = |src_path: &Path| {
                    let source = read_source_file(src_path).map_err(|err| {
                        Diagnostic::spans_error(
                            self.span,
                            format!("Error reading file {path:?}: {err}"),
                        )
                    })?;
                    self.compile_string(&source, mode, module_name, compiler, || {
                        path.strip_prefix(base).ok().unwrap_or(&path).display()
                    })
                };
                let code = compile_path(&path).or_else(|e| {
                    if cfg!(windows)
                        && let Ok(real_path) = fs::read_to_string(path.canonicalize().unwrap())
                    {
                        let joined = path.parent().unwrap().join(real_path.trim());
                        if joined.exists() {
                            return compile_path(&joined);
                        }
                        return Err(e);
                    }
                    Err(e)
                });

                let code = match code {
                    Ok(code) => code,
                    Err(_)
                        if stem.starts_with("badsyntax_")
                            | parent.ends_with(".encoded_modules") =>
                    {
                        // TODO: handle with macro arg rather than hard-coded path
                        continue;
                    }
                    Err(e) => return Err(e),
                };

                code_map.insert(
                    module_name.to_string(),
                    CompiledModule {
                        code,
                        package: is_init,
                        namespace_container: false,
                    },
                );
            }
        }
        if !parent.is_empty() && !has_initializer && !code_map.is_empty() {
            containers.insert(parent.to_owned(), path.to_owned());
        }
        Ok(code_map)
    }
}

impl PyCompileArgs {
    fn parse(input: TokenStream, allow_dir: bool) -> Result<Self, Diagnostic> {
        let mut module_name = None;
        let mut mode = None;
        let mut source: Option<CompilationSource> = None;
        let mut crate_name = None;

        fn assert_source_empty(source: Option<&CompilationSource>) -> Result<(), syn::Error> {
            if let Some(source) = source {
                Err(syn::Error::new(
                    source.span.0,
                    "Cannot have more than one source",
                ))
            } else {
                Ok(())
            }
        }

        syn::meta::parser(|meta| {
            let ident = meta
                .path
                .get_ident()
                .ok_or_else(|| meta.error("unknown arg"))?;
            let check_str = || meta.value()?.call(parse_str);
            let str_path = || {
                let s = check_str()?;
                let mut base_path = s
                    .span()
                    .unwrap()
                    .local_file()
                    .ok_or_else(|| err_span!(s, "filepath literal has no span information"))?;
                base_path.pop();
                Ok::<_, syn::Error>((base_path, PathBuf::from(s.value())))
            };
            if ident == "mode" {
                let s = check_str()?;
                match s.value().parse() {
                    Ok(mode_val) => mode = Some(mode_val),
                    Err(e) => bail_span!(s, "{}", e),
                }
            } else if ident == "module_name" {
                module_name = Some(check_str()?.value())
            } else if ident == "source" {
                assert_source_empty(source.as_ref())?;
                let code = check_str()?.value();
                source = Some(CompilationSource {
                    kind: CompilationSourceKind::SourceCode(code),
                    span: (ident.span(), meta.input.cursor().span()),
                });
            } else if ident == "file" {
                assert_source_empty(source.as_ref())?;
                let (base, rel_path) = str_path()?;
                source = Some(CompilationSource {
                    kind: CompilationSourceKind::File { base, rel_path },
                    span: (ident.span(), meta.input.cursor().span()),
                });
            } else if ident == "dir" {
                if !allow_dir {
                    bail_span!(ident, "py_compile doesn't accept dir")
                }

                assert_source_empty(source.as_ref())?;
                let (base, rel_path) = str_path()?;
                source = Some(CompilationSource {
                    kind: CompilationSourceKind::Dir { base, rel_path },
                    span: (ident.span(), meta.input.cursor().span()),
                });
            } else if ident == "crate_name" {
                let name = check_str()?.parse()?;
                crate_name = Some(name);
            } else {
                return Err(meta.error("unknown attr"));
            }
            Ok(())
        })
        .parse2(input)?;

        let source = source.ok_or_else(|| {
            syn::Error::new(
                Span::call_site(),
                "Must have either file or source in py_compile!()/py_freeze!()",
            )
        })?;

        Ok(Self {
            source,
            mode: mode.unwrap_or(Mode::Exec),
            module_name: module_name.unwrap_or_else(|| "frozen".to_owned()),
            crate_name: crate_name.unwrap_or_else(|| syn::parse_quote!(::rustpython_vm)),
        })
    }
}

fn parse_str(input: ParseStream<'_>) -> ParseResult<LitStr> {
    let span = input.span();
    if input.peek(LitStr) {
        input.parse()
    } else if let Ok(mac) = input.parse::<Macro>() {
        Ok(LitStr::new(&mac.tokens.to_string(), mac.span()))
    } else {
        Err(syn::Error::new(span, "Expected string or stringify macro"))
    }
}

struct PyCompileArgs {
    source: CompilationSource,
    mode: Mode,
    module_name: String,
    crate_name: syn::Path,
}

pub(crate) fn impl_py_compile(
    input: TokenStream,
    compiler: &dyn Compiler,
) -> Result<TokenStream, Diagnostic> {
    let args = PyCompileArgs::parse(input, false)?;

    let crate_name = args.crate_name;
    let code = args
        .source
        .compile_single(args.mode, &args.module_name, compiler)?;

    let frozen = frozen::FrozenCodeObject::encode(&code);
    let bytes = LitByteStr::new(&frozen.bytes, Span::call_site());

    let output = quote! {
        #crate_name::frozen::FrozenCodeObject { bytes: &#bytes[..] }
    };

    Ok(output)
}

pub(crate) fn impl_py_freeze(
    input: TokenStream,
    compiler: &dyn Compiler,
) -> Result<TokenStream, Diagnostic> {
    let args = PyCompileArgs::parse(input, true)?;

    let crate_name = args.crate_name;
    let code_map = args
        .source
        .compile(args.mode, &args.module_name, compiler)?;

    let data = frozen::FrozenLib::encode(code_map.iter().map(|(k, v)| {
        let v = frozen::FrozenModule {
            code: frozen::FrozenCodeObject::encode(&v.code),
            package: v.package,
            namespace_container: v.namespace_container,
        };
        (&**k, v)
    }));
    let bytes = LitByteStr::new(&data.bytes, Span::call_site());

    let output = quote! {
        #crate_name::frozen::FrozenLib::from_ref(#bytes)
    };

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use rustpython_compiler_core::bytecode::{CodeFlags, ConstantData, Constants};

    struct FixtureCompiler;

    impl Compiler for FixtureCompiler {
        fn compile(
            &self,
            source: &str,
            _mode: Mode,
            module_name: &str,
        ) -> Result<CodeObject, Box<dyn core::error::Error>> {
            if source == "invalid fixture" {
                return Err(std::io::Error::other("fixture excluded by existing policy").into());
            }
            // Structural generator fixture only. No Python source is compiled
            // or executed; source_path records which real entry survived.
            Ok(CodeObject {
                instructions: Vec::new().into(),
                locations: Box::new([]),
                flags: CodeFlags::empty(),
                posonlyarg_count: 0,
                arg_count: 0,
                kwonlyarg_count: 0,
                source_path: source.to_owned(),
                first_line_number: None,
                max_stackdepth: 0,
                obj_name: module_name.to_owned(),
                qualname: module_name.to_owned(),
                constants: Constants::<ConstantData>::new(Box::new([])),
                names: Box::new([]),
                varnames: Box::new([]),
                cellvars: Box::new([]),
                freevars: Box::new([]),
                localspluskinds: Box::new([]),
                linetable: Box::new([]),
                exceptiontable: Box::new([]),
            })
        }
    }

    struct FixtureDir(PathBuf);

    impl FixtureDir {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "rustpython-freeze-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn write(&self, path: &str, source: &str) {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source).unwrap();
        }

        fn compile(&self) -> HashMap<String, CompiledModule> {
            let source = CompilationSource {
                kind: CompilationSourceKind::Dir {
                    base: self.0.clone(),
                    rel_path: PathBuf::new(),
                },
                span: (Span::call_site(), Span::call_site()),
            };
            source
                .compile(Mode::Exec, "", &FixtureCompiler)
                .unwrap_or_else(|_| panic!("fixture compilation failed"))
        }
    }

    impl Drop for FixtureDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn frozen_containers_follow_physical_directories_and_preserve_real_entries() {
        for module_first in [true, false] {
            let dir = FixtureDir::new();
            if module_first {
                dir.write("collision.py", "real module");
            }
            dir.write("collision/child.py", "child");
            if !module_first {
                dir.write("collision.py", "real module");
            }
            dir.write("namespace/nested/child.py", "child");
            dir.write("namespace/regular/__init__.py", "real initializer");
            dir.write("dotted.name/child.py", "child");
            dir.write("data/readme.txt", "data only");
            fs::create_dir(dir.0.join("empty")).unwrap();
            dir.write("excluded/badsyntax_fixture.py", "invalid fixture");
            dir.write("test/encoded_modules/__init__.py", "invalid fixture");
            dir.write("test/encoded_modules/child.py", "child");
            let modules = dir.compile();
            for name in ["namespace", "namespace.nested", "dotted.name"] {
                assert!(modules[name].package);
                assert!(modules[name].namespace_container);
                assert_eq!(modules[name].code.source_path, "");
            }
            assert!(!modules["collision"].package);
            assert!(!modules["collision"].namespace_container);
            assert_eq!(modules["collision"].code.source_path, "real module");
            assert!(modules["namespace.regular"].package);
            assert!(!modules["namespace.regular"].namespace_container);
            assert_eq!(
                modules["namespace.regular"].code.source_path,
                "real initializer"
            );
            for absent in [
                "",
                "dotted",
                "empty",
                "data",
                "excluded",
                "test.encoded_modules",
            ] {
                assert!(!modules.contains_key(absent), "unexpected entry {absent}");
            }
        }
    }
}
