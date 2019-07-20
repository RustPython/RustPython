//! Parsing and processing for this form:
//! ```ignore
//! py_compile_input!(
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
use bincode;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use rustpython_bytecode::bytecode::CodeObject;
use rustpython_compiler::compile;
use std::env;
use std::fs;
use std::path::PathBuf;
use syn::parse::{Parse, ParseStream, Result as ParseResult};
use syn::{self, parse2, Lit, LitByteStr, LitStr, Meta, Token};

enum CompilationSourceKind {
    File(PathBuf),
    SourceCode(String),
}

struct CompilationSource {
    kind: CompilationSourceKind,
    span: (Span, Span),
}

impl CompilationSource {
    fn compile(self, mode: &compile::Mode, module_name: String) -> Result<CodeObject, Diagnostic> {
        let compile = |source| {
            compile::compile(source, mode, module_name, 0).map_err(|err| {
                Diagnostic::spans_error(self.span, format!("Compile error: {}", err))
            })
        };

        match &self.kind {
            CompilationSourceKind::File(rel_path) => {
                let mut path = PathBuf::from(
                    env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not present"),
                );
                path.push(rel_path);
                let source = fs::read_to_string(&path).map_err(|err| {
                    Diagnostic::spans_error(
                        self.span,
                        format!("Error reading file {:?}: {}", path, err),
                    )
                })?;
                compile(&source)
            }
            CompilationSourceKind::SourceCode(code) => compile(code),
        }
    }
}

/// This is essentially just a comma-separated list of Meta nodes, aka the inside of a MetaList.
struct PyCompileInput {
    span: Span,
    metas: Vec<Meta>,
}

impl PyCompileInput {
    fn compile(&self) -> Result<CodeObject, Diagnostic> {
        let mut module_name = None;
        let mut mode = None;
        let mut source: Option<CompilationSource> = None;

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
                if name_value.ident == "mode" {
                    mode = Some(match &name_value.lit {
                        Lit::Str(s) => match s.value().as_str() {
                            "exec" => compile::Mode::Exec,
                            "eval" => compile::Mode::Eval,
                            "single" => compile::Mode::Single,
                            _ => bail_span!(s, "mode must be exec, eval, or single"),
                        },
                        _ => bail_span!(name_value.lit, "mode must be a string"),
                    })
                } else if name_value.ident == "module_name" {
                    module_name = Some(match &name_value.lit {
                        Lit::Str(s) => s.value(),
                        _ => bail_span!(name_value.lit, "module_name must be string"),
                    })
                } else if name_value.ident == "source" {
                    assert_source_empty(&source)?;
                    let code = match &name_value.lit {
                        Lit::Str(s) => s.value(),
                        _ => bail_span!(name_value.lit, "source must be a string"),
                    };
                    source = Some(CompilationSource {
                        kind: CompilationSourceKind::SourceCode(code),
                        span: extract_spans(&name_value).unwrap(),
                    });
                } else if name_value.ident == "file" {
                    assert_source_empty(&source)?;
                    let path = match &name_value.lit {
                        Lit::Str(s) => PathBuf::from(s.value()),
                        _ => bail_span!(name_value.lit, "source must be a string"),
                    };
                    source = Some(CompilationSource {
                        kind: CompilationSourceKind::File(path),
                        span: extract_spans(&name_value).unwrap(),
                    });
                }
            }
        }

        source
            .ok_or_else(|| {
                Diagnostic::span_error(
                    self.span,
                    "Must have either file or source in py_compile_bytecode!()",
                )
            })?
            .compile(
                &mode.unwrap_or(compile::Mode::Exec),
                module_name.unwrap_or_else(|| "frozen".to_string()),
            )
    }
}

impl Parse for PyCompileInput {
    fn parse(input: ParseStream) -> ParseResult<Self> {
        let span = input.cursor().span();
        let metas = input
            .parse_terminated::<Meta, Token![,]>(Meta::parse)?
            .into_iter()
            .collect();
        Ok(PyCompileInput { span, metas })
    }
}

pub fn impl_py_compile_bytecode(input: TokenStream2) -> Result<TokenStream2, Diagnostic> {
    let input: PyCompileInput = parse2(input)?;

    let code_obj = input.compile()?;

    let module_name = LitStr::new(&code_obj.source_path, Span::call_site());

    let bytes = bincode::serialize(&code_obj).expect("Failed to serialize");
    let bytes = LitByteStr::new(&bytes, Span::call_site());

    let output = quote! {
        ({
            use ::rustpython_vm::__exports::bincode;
            hashmap! { #module_name.into() => bincode::deserialize::<::rustpython_vm::bytecode::CodeObject>(#bytes)
                .expect("Deserializing CodeObject failed")}
        })
    };

    Ok(output)
}
