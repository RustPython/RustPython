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

use crate::{extract_spans, Diagnostic};
use once_cell::sync::Lazy;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use rustpython_bytecode::{CodeObject, FrozenModule};
use rustpython_compiler as compile;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use syn::parse::{Parse, ParseStream, Result as ParseResult};
use syn::spanned::Spanned;
use syn::{self, parse2, Lit, LitByteStr, LitStr, Macro, Meta, MetaNameValue, Token};

static CARGO_MANIFEST_DIR: Lazy<PathBuf> = Lazy::new(|| {
    PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not present"))
});

enum CompilationSourceKind {
    File(PathBuf),
    SourceCode(String),
    Dir(PathBuf),
}

struct CompilationSource {
    kind: CompilationSourceKind,
    span: (Span, Span),
}

impl CompilationSource {
    fn compile_string<D: std::fmt::Display, F: FnOnce() -> D>(
        &self,
        source: &str,
        mode: compile::Mode,
        module_name: String,
        origin: F,
    ) -> Result<CodeObject, Diagnostic> {
        compile::compile(source, mode, module_name, compile::CompileOpts::default()).map_err(
            |err| {
                Diagnostic::spans_error(
                    self.span,
                    format!("Python compile error from {}: {}", origin(), err),
                )
            },
        )
    }

    fn compile(
        &self,
        mode: compile::Mode,
        module_name: String,
    ) -> Result<HashMap<String, FrozenModule>, Diagnostic> {
        match &self.kind {
            CompilationSourceKind::Dir(rel_path) => {
                self.compile_dir(&CARGO_MANIFEST_DIR.join(rel_path), String::new(), mode)
            }
            _ => Ok(hashmap! {
                module_name.clone() => FrozenModule {
                    code: self.compile_single(mode, module_name)?,
                    package: false,
                },
            }),
        }
    }

    fn compile_single(
        &self,
        mode: compile::Mode,
        module_name: String,
    ) -> Result<CodeObject, Diagnostic> {
        match &self.kind {
            CompilationSourceKind::File(rel_path) => {
                let path = CARGO_MANIFEST_DIR.join(rel_path);
                let source = fs::read_to_string(&path).map_err(|err| {
                    Diagnostic::spans_error(
                        self.span,
                        format!("Error reading file {:?}: {}", path, err),
                    )
                })?;
                self.compile_string(&source, mode, module_name, || rel_path.display())
            }
            CompilationSourceKind::SourceCode(code) => {
                self.compile_string(&textwrap::dedent(code), mode, module_name, || {
                    "string literal"
                })
            }
            CompilationSourceKind::Dir(_) => {
                unreachable!("Can't use compile_single with directory source")
            }
        }
    }

    fn compile_dir(
        &self,
        path: &Path,
        parent: String,
        mode: compile::Mode,
    ) -> Result<HashMap<String, FrozenModule>, Diagnostic> {
        let mut code_map = HashMap::new();
        let paths = fs::read_dir(&path).map_err(|err| {
            Diagnostic::spans_error(self.span, format!("Error listing dir {:?}: {}", path, err))
        })?;
        for path in paths {
            let path = path.map_err(|err| {
                Diagnostic::spans_error(self.span, format!("Failed to list file: {}", err))
            })?;
            let path = path.path();
            let file_name = path.file_name().unwrap().to_str().ok_or_else(|| {
                Diagnostic::spans_error(self.span, format!("Invalid UTF-8 in file name {:?}", path))
            })?;
            if path.is_dir() {
                code_map.extend(self.compile_dir(
                    &path,
                    format!("{}{}", parent, file_name),
                    mode,
                )?);
            } else if file_name.ends_with(".py") {
                let source = fs::read_to_string(&path).map_err(|err| {
                    Diagnostic::spans_error(
                        self.span,
                        format!("Error reading file {:?}: {}", path, err),
                    )
                })?;
                let stem = path.file_stem().unwrap().to_str().unwrap();
                let is_init = stem == "__init__";
                let module_name = if is_init {
                    parent.clone()
                } else if parent.is_empty() {
                    stem.to_owned()
                } else {
                    format!("{}.{}", parent, stem)
                };
                code_map.insert(
                    module_name.clone(),
                    FrozenModule {
                        code: self.compile_string(&source, mode, module_name, || {
                            path.strip_prefix(&*CARGO_MANIFEST_DIR)
                                .ok()
                                .unwrap_or(&path)
                                .display()
                        })?,
                        package: is_init,
                    },
                );
            }
        }
        Ok(code_map)
    }
}

/// This is essentially just a comma-separated list of Meta nodes, aka the inside of a MetaList.
struct PyCompileInput {
    span: Span,
    metas: Vec<Meta>,
}

impl PyCompileInput {
    fn parse(&self, allow_dir: bool) -> Result<PyCompileArgs, Diagnostic> {
        let mut module_name = None;
        let mut mode = None;
        let mut source: Option<CompilationSource> = None;
        let mut crate_name = None;

        fn assert_source_empty(source: &Option<CompilationSource>) -> Result<(), Diagnostic> {
            if let Some(source) = source {
                Err(Diagnostic::spans_error(
                    source.span,
                    "Cannot have more than one source",
                ))
            } else {
                Ok(())
            }
        }

        for meta in &self.metas {
            if let Meta::NameValue(name_value) = meta {
                let ident = match name_value.path.get_ident() {
                    Some(ident) => ident,
                    None => continue,
                };
                if ident == "mode" {
                    match &name_value.lit {
                        Lit::Str(s) => match s.value().parse() {
                            Ok(mode_val) => mode = Some(mode_val),
                            Err(e) => bail_span!(s, "{}", e),
                        },
                        _ => bail_span!(name_value.lit, "mode must be a string"),
                    }
                } else if ident == "module_name" {
                    module_name = Some(match &name_value.lit {
                        Lit::Str(s) => s.value(),
                        _ => bail_span!(name_value.lit, "module_name must be string"),
                    })
                } else if ident == "source" {
                    assert_source_empty(&source)?;
                    let code = match &name_value.lit {
                        Lit::Str(s) => s.value(),
                        _ => bail_span!(name_value.lit, "source must be a string"),
                    };
                    source = Some(CompilationSource {
                        kind: CompilationSourceKind::SourceCode(code),
                        span: extract_spans(&name_value).unwrap(),
                    });
                } else if ident == "file" {
                    assert_source_empty(&source)?;
                    let path = match &name_value.lit {
                        Lit::Str(s) => PathBuf::from(s.value()),
                        _ => bail_span!(name_value.lit, "source must be a string"),
                    };
                    source = Some(CompilationSource {
                        kind: CompilationSourceKind::File(path),
                        span: extract_spans(&name_value).unwrap(),
                    });
                } else if ident == "dir" {
                    if !allow_dir {
                        bail_span!(ident, "py_compile doesn't accept dir")
                    }

                    assert_source_empty(&source)?;
                    let path = match &name_value.lit {
                        Lit::Str(s) => PathBuf::from(s.value()),
                        _ => bail_span!(name_value.lit, "source must be a string"),
                    };
                    source = Some(CompilationSource {
                        kind: CompilationSourceKind::Dir(path),
                        span: extract_spans(&name_value).unwrap(),
                    });
                } else if ident == "crate_name" {
                    let name = match &name_value.lit {
                        Lit::Str(s) => s.parse()?,
                        _ => bail_span!(name_value.lit, "source must be a string"),
                    };
                    crate_name = Some(name);
                }
            }
        }

        let source = source.ok_or_else(|| {
            Diagnostic::span_error(
                self.span,
                "Must have either file or source in py_compile!()/py_freeze!()",
            )
        })?;

        Ok(PyCompileArgs {
            source,
            mode: mode.unwrap_or(compile::Mode::Exec),
            module_name: module_name.unwrap_or_else(|| "frozen".to_owned()),
            crate_name: crate_name.unwrap_or_else(|| syn::parse_quote!(::rustpython_vm::bytecode)),
        })
    }
}

fn parse_meta(input: ParseStream) -> ParseResult<Meta> {
    let path = input.call(syn::Path::parse_mod_style)?;
    let eq_token: Token![=] = input.parse()?;
    let span = input.span();
    if input.peek(LitStr) {
        Ok(Meta::NameValue(MetaNameValue {
            path,
            eq_token,
            lit: Lit::Str(input.parse()?),
        }))
    } else if let Ok(mac) = input.parse::<Macro>() {
        Ok(Meta::NameValue(MetaNameValue {
            path,
            eq_token,
            lit: Lit::Str(LitStr::new(&mac.tokens.to_string(), mac.span())),
        }))
    } else {
        Err(syn::Error::new(span, "Expected string or stringify macro"))
    }
}

impl Parse for PyCompileInput {
    fn parse(input: ParseStream) -> ParseResult<Self> {
        let span = input.cursor().span();
        let metas = input
            .parse_terminated::<Meta, Token![,]>(parse_meta)?
            .into_iter()
            .collect();
        Ok(PyCompileInput { span, metas })
    }
}

struct PyCompileArgs {
    source: CompilationSource,
    mode: compile::Mode,
    module_name: String,
    crate_name: syn::Path,
}

pub fn impl_py_compile(input: TokenStream2) -> Result<TokenStream2, Diagnostic> {
    let input: PyCompileInput = parse2(input)?;
    let args = input.parse(false)?;

    let crate_name = args.crate_name;
    let code = args.source.compile_single(args.mode, args.module_name)?;

    let bytes = code.to_bytes();
    let bytes = LitByteStr::new(&bytes, Span::call_site());

    let output = quote! {
        #crate_name::CodeObject::from_bytes(#bytes)
            .expect("Deserializing CodeObject failed")
    };

    Ok(output)
}

pub fn impl_py_freeze(input: TokenStream2) -> Result<TokenStream2, Diagnostic> {
    let input: PyCompileInput = parse2(input)?;
    let args = input.parse(true)?;

    let crate_name = args.crate_name;
    let code_map = args.source.compile(args.mode, args.module_name)?;

    let data = rustpython_bytecode::frozen_lib::encode_lib(code_map.iter().map(|(k, v)| (&**k, v)));
    let bytes = LitByteStr::new(&data, Span::call_site());

    let output = quote! {
        #crate_name::frozen_lib::decode_lib(#bytes)
    };

    Ok(output)
}
