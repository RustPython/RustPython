use super::Diagnostic;
use bincode;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use rustpython_compiler::{bytecode::CodeObject, compile};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use syn::parse::{Parse, ParseStream, Result as ParseResult};
use syn::{self, parse2, Ident, Lit, LitByteStr, Meta, MetaList, NestedMeta, Token};

struct BytecodeConst {
    ident: Ident,
    meta: MetaList,
}

impl BytecodeConst {
    fn compile(&self, manifest_dir: &Path) -> Result<CodeObject, Diagnostic> {
        let meta = &self.meta;

        let mut source_path = None;
        let mut mode = None;
        let mut source_lit = None;

        for meta in &meta.nested {
            match meta {
                NestedMeta::Literal(lit) => source_lit = Some(lit),
                NestedMeta::Meta(Meta::NameValue(name_value)) => {
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
                    }
                }
                _ => {}
            }
        }

        let source = if meta.ident == "file" {
            let path = match source_lit {
                Some(Lit::Str(s)) => s.value(),
                _ => bail_span!(source_lit, "Expected string literal for path to file()"),
            };
            let path = manifest_dir.join(path);
            fs::read_to_string(&path)
                .map_err(|err| err_span!(source_lit, "Error reading file {:?}: {}", path, err))?
        } else if meta.ident == "source" {
            match source_lit {
                Some(Lit::Str(s)) => s.value(),
                _ => bail_span!(source_lit, "Expected string literal for source()"),
            }
        } else {
            bail_span!(meta.ident, "Expected either 'file' or 'source'")
        };

        compile::compile(
            &source,
            &mode.unwrap_or(compile::Mode::Exec),
            source_path.unwrap_or_else(|| "".to_string()),
        )
        .map_err(|err| err_span!(source_lit, "Compile error: {}", err))
    }
}

impl Parse for BytecodeConst {
    /// Parse the form `static ref IDENT = metalist(...);`
    fn parse(input: ParseStream) -> ParseResult<Self> {
        input.parse::<Token![static]>()?;
        input.parse::<Token![ref]>()?;
        let ident = input.parse()?;
        input.parse::<Token![=]>()?;
        let meta = input.parse()?;
        input.parse::<Token![;]>()?;
        Ok(BytecodeConst { ident, meta })
    }
}

struct PyCompileInput(Vec<BytecodeConst>);

impl Parse for PyCompileInput {
    fn parse(input: ParseStream) -> ParseResult<Self> {
        std::iter::from_fn(|| {
            if input.is_empty() {
                None
            } else {
                Some(input.parse())
            }
        })
        .collect::<ParseResult<_>>()
        .map(PyCompileInput)
    }
}

pub fn impl_py_compile_bytecode(input: TokenStream2) -> Result<TokenStream2, Diagnostic> {
    let PyCompileInput(consts) = parse2(input)?;

    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not present"),
    );

    let consts = consts
        .into_iter()
        .map(|bytecode_const| -> Result<_, Diagnostic> {
            let code_obj = bytecode_const.compile(&manifest_dir)?;
            let ident = bytecode_const.ident;
            let bytes = bincode::serialize(&code_obj).expect("Failed to serialize");
            let bytes = LitByteStr::new(&bytes, Span::call_site());
            Ok(quote! {
                static ref #ident: ::rustpython_vm::bytecode::CodeObject = {
                    use bincode;
                    bincode::deserialize(#bytes).expect("Deserializing CodeObject failed")
                };
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let output = quote! {
        lazy_static! {
            #(#consts)*
        }
    };

    Ok(output)
}
