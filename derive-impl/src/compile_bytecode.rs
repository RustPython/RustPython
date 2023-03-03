//! Parsing and processing for this form:
//! ```ignore
//! py_compile!(
//!     // either:
//!     source = "python_source_code",
//!     // or
//!     file = "file/path/relative/to/$CARGO_MANIFEST_DIR",
//!
//!     // the mode to compile the code in
//!     mode = "exec", // or "eval" or "single"
//!     // the path put into the CodeObject, defaults to "frozen"
//!     module_name = "frozen",
//! )
//! ```

use crate::Diagnostic;
use proc_macro2::{Span, TokenStream};
use quote::quote;
use rustpython_compiler_core::{Mode, bytecode::CodeObject, frozen};
use std::sync::LazyLock;
use std::{
    collections::BTreeMap,
    env, fs,
    ops::Not,
    path::{Path, PathBuf},
};
use syn::{
    self, LitByteStr, LitStr, Macro,
    parse::{ParseStream, Parser, Result as ParseResult},
    spanned::Spanned,
};

static CARGO_MANIFEST_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
    PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not present"))
});
fn resolve_path(path: &Path) -> std::borrow::Cow<'_, Path> {
    if path.is_absolute() {
        path.into()
    } else {
        CARGO_MANIFEST_DIR.join(path).into()
    }
}

enum CompilationSource {
    /// Source is a File (Path)
    File(PathBuf),
    /// Direct Raw source code
    SourceCode(String),
    /// Source is a directory
    Dir(PathBuf),
}

struct CompiledModule {
    code: CodeObject,
    package: bool,
}

pub trait Compiler {
    fn compile(
        &self,
        source: &str,
        mode: Mode,
        source_path: String,
    ) -> Result<CodeObject, Box<dyn std::error::Error>>;
}

impl CompilationSource {
    fn compile_string(
        source: &str,
        mode: Mode,
        module_name: &str,
        compiler: &dyn Compiler,
    ) -> Result<CodeObject, Box<dyn std::error::Error>> {
        compiler.compile(source, mode, format!("<frozen {module_name}>"))
    }

    fn compile(
        &self,
        mode: Mode,
        module_name: String,
        compiler: &dyn Compiler,
    ) -> Result<Vec<(String, CompiledModule)>, String> {
        match self {
            CompilationSource::Dir(path) => DirWalker::from_dir(&resolve_path(path))?
                .modules
                .into_iter()
                .map(|(module_name, (path, package))| {
                    let module = Self::compile_file(&path, mode, &module_name, compiler)
                        .map(|code| CompiledModule { code, package });
                    (module_name, module)
                })
                .filter_map(|(module_name, res)| {
                    let is_bad_syntax = res.is_err() && {
                        let (parent, stem) =
                            module_name.rsplit_once('.').unwrap_or(("", &module_name));
                        // TODO: handle with macro arg rather than hard-coded path
                        stem.starts_with("badsyntax_") || parent.ends_with(".encoded_modules")
                    };
                    is_bad_syntax.not().then(|| Ok((module_name, res?)))
                })
                .collect(),
            _ => {
                let module = CompiledModule {
                    code: self.compile_single(mode, &module_name, compiler)?,
                    package: false,
                };
                Ok(vec![(module_name, module)])
            }
        }
    }

    fn compile_file(
        path: &Path,
        mode: Mode,
        module_name: &str,
        compiler: &dyn Compiler,
    ) -> Result<CodeObject, String> {
        let compile_path = |src_path: &Path| {
            let source = fs::read_to_string(resolve_path(src_path))
                .map_err(|err| format!("Error reading file {path:?}: {err}"))?;
            Self::compile_string(&source, mode, module_name, compiler).map_err(|err| {
                let rel_path = path.strip_prefix(&*CARGO_MANIFEST_DIR).unwrap_or(path);
                format!("Python compile error in {}: {err}", rel_path.display())
            })
        };
        compile_path(path).or_else(|e| {
            if cfg!(windows) {
                if let Ok(real_path) = fs::read_to_string(path.canonicalize().unwrap()) {
                    let joined = path.parent().unwrap().join(real_path.trim());
                    if joined.exists() {
                        return compile_path(&joined);
                    } else {
                        return Err(e);
                    }
                }
            }
            Err(e)
        })
    }

    fn compile_single(
        &self,
        mode: Mode,
        module_name: &str,
        compiler: &dyn Compiler,
    ) -> Result<CodeObject, String> {
        match self {
            CompilationSource::File(path) => Self::compile_file(path, mode, module_name, compiler),
            CompilationSource::SourceCode(code) => {
                Self::compile_string(&textwrap::dedent(code), mode, module_name, compiler)
                    .map_err(|err| format!("Python compile error in string literal: {err}"))
            }
            CompilationSource::Dir(_) => {
                unreachable!("Can't use compile_single with directory source")
            }
        }
    }
}

#[derive(Default)]
struct DirWalker {
    modules: BTreeMap<String, (PathBuf, bool)>,
}

impl DirWalker {
    fn from_dir(path: &Path) -> Result<Self, String> {
        let mut dir = Self::default();
        dir.walk(path, "")?;
        Ok(dir)
    }
    fn walk(&mut self, path: &Path, parent: &str) -> Result<(), String> {
        let paths = fs::read_dir(path)
            .or_else(|e| {
                if cfg!(windows) {
                    if let Ok(real_path) = fs::read_to_string(path.canonicalize().unwrap()) {
                        return fs::read_dir(real_path.trim());
                    }
                }
                Err(e)
            })
            .map_err(|err| format!("Error listing dir {path:?}: {err}"))?;
        for path in paths {
            let path = path.map_err(|err| format!("Failed to list file: {err}"))?;
            self.add_entry(path.path(), parent)?;
        }
        Ok(())
    }
    fn add_entry(&mut self, path: PathBuf, parent: &str) -> Result<(), String> {
        let file_name = path
            .file_name()
            .unwrap()
            .to_str()
            .ok_or_else(|| format!("Invalid UTF-8 in file name {path:?}"))?;
        if path.is_dir() {
            if parent.is_empty() {
                self.walk(&path, file_name)?
            } else {
                self.walk(&path, &[parent, ".", file_name].concat())?
            }
        } else if file_name.ends_with(".py") {
            let stem = path.file_stem().unwrap().to_str().unwrap();
            let is_init = stem == "__init__";
            let module_name = if is_init {
                parent.to_owned()
            } else if parent.is_empty() {
                stem.to_owned()
            } else {
                [parent, ".", stem].concat()
            };

            self.modules.insert(module_name, (path, is_init));
        }
        Ok(())
    }
}

impl PyCompileArgs {
    fn parse(input: TokenStream, allow_dir: bool) -> Result<PyCompileArgs, Diagnostic> {
        let mut module_name = None;
        let mut mode = None;
        let mut source: Option<CompilationSource> = None;
        let mut source_span = (Span::call_site(), Span::call_site());
        let mut crate_name = None;

        syn::meta::parser(|meta| {
            let assert_source_empty = || {
                if source.is_some() {
                    Err(meta.error("Cannot have more than one source"))
                } else {
                    Ok(())
                }
            };
            let ident = meta
                .path
                .get_ident()
                .ok_or_else(|| meta.error("unknown arg"))?;
            let check_str = || meta.value()?.call(parse_str);
            if ident == "mode" {
                let s = check_str()?;
                match s.value().parse() {
                    Ok(mode_val) => mode = Some(mode_val),
                    Err(e) => bail_span!(s, "{}", e),
                }
            } else if ident == "module_name" {
                module_name = Some(check_str()?.value())
            } else if ident == "source" {
                assert_source_empty()?;
                let code = check_str()?.value();
                source_span = (ident.span(), code.span());
                source = Some(CompilationSource::SourceCode(code));
            } else if ident == "file" {
                assert_source_empty()?;
                let path = check_str()?;
                source_span = (ident.span(), path.span());
                source = Some(CompilationSource::File(path.value().into()));
            } else if ident == "dir" {
                if !allow_dir {
                    bail_span!(ident, "py_compile doesn't accept dir")
                }

                assert_source_empty()?;
                let path = check_str()?;
                source_span = (ident.span(), path.span());
                source = Some(CompilationSource::Dir(path.value().into()));
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

        Ok(PyCompileArgs {
            source,
            source_span,
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
    source_span: (Span, Span),
    mode: Mode,
    module_name: String,
    crate_name: syn::Path,
}

pub fn impl_py_compile(
    input: TokenStream,
    compiler: &dyn Compiler,
) -> Result<TokenStream, Diagnostic> {
    let args = PyCompileArgs::parse(input, false)?;

    let crate_name = args.crate_name;
    let code = args
        .source
        .compile_single(args.mode, &args.module_name, compiler)
        .map_err(|msg| Diagnostic::spans_error(args.source_span, msg))?;

    let frozen = frozen::FrozenCodeObject::encode(&code);
    let bytes = LitByteStr::new(&frozen.bytes, Span::call_site());

    let output = quote! {
        #crate_name::frozen::FrozenCodeObject { bytes: &#bytes[..] }
    };

    Ok(output)
}

pub fn impl_py_freeze(
    input: TokenStream,
    compiler: &dyn Compiler,
) -> Result<TokenStream, Diagnostic> {
    let args = PyCompileArgs::parse(input, true)?;

    let crate_name = args.crate_name;
    let code_map = args
        .source
        .compile(args.mode, args.module_name, compiler)
        .map_err(|msg| Diagnostic::spans_error(args.source_span, msg))?;

    let data = frozen::FrozenLib::encode(code_map.iter().map(|(k, v)| {
        let v = frozen::FrozenModule {
            code: frozen::FrozenCodeObject::encode(&v.code),
            package: v.package,
        };
        (&**k, v)
    }));
    let bytes = LitByteStr::new(&data.bytes, Span::call_site());

    let output = quote! {
        #crate_name::frozen::FrozenLib::from_ref(#bytes)
    };

    Ok(output)
}
