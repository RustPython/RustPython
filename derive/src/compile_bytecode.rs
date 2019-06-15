use crate::{extract_spans, Diagnostic};
use bincode;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use rustpython_compiler::{bytecode::CodeObject, compile};
use std::env;
use std::fs;
use std::path::PathBuf;
use syn::parse::{Parse, ParseStream, Result as ParseResult};
use syn::{self, parse2, Lit, LitByteStr, Meta, Token};

enum CompilationSourceKind {
    File(PathBuf),
    SourceCode(String),
}

struct CompilationSource {
    kind: CompilationSourceKind,
    span: (Span, Span),
}

impl CompilationSource {
    fn compile(self, mode: &compile::Mode, source_path: String) -> Result<CodeObject, Diagnostic> {
        let compile = |source| {
            compile::compile(source, mode, source_path).map_err(|err| {
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

struct PyCompileInput {
    span: Span,
    metas: Vec<Meta>,
}

struct PyCompileResult {
    code_obj: CodeObject,
    lazy_static: bool,
}

impl PyCompileInput {
    fn compile(&self) -> Result<PyCompileResult, Diagnostic> {
        let mut source_path = None;
        let mut mode = None;
        let mut source: Option<CompilationSource> = None;
        let mut lazy_static = false;

        fn assert_source_empty(source: &Option<CompilationSource>) -> Result<(), Diagnostic> {
            if let Some(source) = source {
                Err(Diagnostic::spans_error(
                    source.span.clone(),
                    "Cannot have more than one source",
                ))
            } else {
                Ok(())
            }
        }

        for meta in &self.metas {
            match meta {
                Meta::NameValue(name_value) => {
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
                    } else if name_value.ident == "source_path" {
                        source_path = Some(match &name_value.lit {
                            Lit::Str(s) => s.value(),
                            _ => bail_span!(name_value.lit, "source_path must be string"),
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
                Meta::Word(ident) => {
                    if ident == "lazy_static" {
                        lazy_static = true;
                    }
                }
                _ => {}
            }
        }

        let code_obj = source
            .ok_or_else(|| {
                Diagnostic::span_error(
                    self.span.clone(),
                    "Must have either file or source in py_compile_bytecode!()",
                )
            })?
            .compile(
                &mode.unwrap_or(compile::Mode::Exec),
                source_path.unwrap_or_else(|| "frozen".to_string()),
            )?;

        Ok(PyCompileResult {
            code_obj,
            lazy_static,
        })
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

    let PyCompileResult {
        code_obj,
        lazy_static,
    } = input.compile()?;

    let bytes = bincode::serialize(&code_obj).expect("Failed to serialize");
    let bytes = LitByteStr::new(&bytes, Span::call_site());

    let output = quote! {
        ({
            use ::bincode;
            bincode::deserialize::<::rustpython_vm::bytecode::CodeObject>(#bytes)
                .expect("Deserializing CodeObject failed")
        })
    };

    if lazy_static {
        Ok(quote! {
            ({
                use ::lazy_static::lazy_static;
                lazy_static! {
                    static ref STATIC: ::rustpython_vm::bytecode::CodeObject = #output;
                }
                &*STATIC
            })
        })
    } else {
        Ok(output)
    }
}
