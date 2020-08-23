// Taken from https://github.com/rustwasm/wasm-bindgen/blob/master/crates/backend/src/error.rs
//
// Copyright (c) 2014 Alex Crichton
//
// Permission is hereby granted, free of charge, to any
// person obtaining a copy of this software and associated
// documentation files (the "Software"), to deal in the
// Software without restriction, including without
// limitation the rights to use, copy, modify, merge,
// publish, distribute, sublicense, and/or sell copies of
// the Software, and to permit persons to whom the Software
// is furnished to do so, subject to the following
// conditions:
//
// The above copyright notice and this permission notice
// shall be included in all copies or substantial portions
// of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
// ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
// TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
// PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
// SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
// CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
// OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
// IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.
#![allow(dead_code)]

use proc_macro2::*;
use quote::{ToTokens, TokenStreamExt};
use syn::parse::Error;

macro_rules! err_span {
    ($span:expr, $($msg:tt)*) => (
        $crate::Diagnostic::spanned_error(&$span, format!($($msg)*))
    )
}

macro_rules! bail_span {
    ($($t:tt)*) => (
        return Err(err_span!($($t)*).into())
    )
}

// macro_rules! push_err_span {
//     ($diagnostics:expr, $($t:tt)*) => {
//         $diagnostics.push(err_span!($($t)*))
//     };
// }

// macro_rules! push_diag_result {
//     ($diags:expr, $x:expr $(,)?) => {
//         if let Err(e) = $x {
//             $diags.push(e);
//         }
//     };
// }

#[derive(Debug)]
pub struct Diagnostic {
    inner: Repr,
}

#[derive(Debug)]
enum Repr {
    Single {
        text: String,
        span: Option<(Span, Span)>,
    },
    SynError(Error),
    Multi {
        diagnostics: Vec<Diagnostic>,
    },
}

impl Diagnostic {
    pub fn error<T: Into<String>>(text: T) -> Diagnostic {
        Diagnostic {
            inner: Repr::Single {
                text: text.into(),
                span: None,
            },
        }
    }

    pub fn span_error<T: Into<String>>(span: Span, text: T) -> Diagnostic {
        Diagnostic {
            inner: Repr::Single {
                text: text.into(),
                span: Some((span, span)),
            },
        }
    }

    pub fn spans_error<T: Into<String>>(spans: (Span, Span), text: T) -> Diagnostic {
        Diagnostic {
            inner: Repr::Single {
                text: text.into(),
                span: Some(spans),
            },
        }
    }

    pub fn spanned_error<T: Into<String>>(node: &dyn ToTokens, text: T) -> Diagnostic {
        Diagnostic {
            inner: Repr::Single {
                text: text.into(),
                span: extract_spans(node),
            },
        }
    }

    pub fn from_vec(diagnostics: Vec<Diagnostic>) -> Result<(), Diagnostic> {
        if diagnostics.is_empty() {
            Ok(())
        } else {
            Err(Diagnostic {
                inner: Repr::Multi { diagnostics },
            })
        }
    }

    #[allow(unconditional_recursion)]
    pub fn panic(&self) -> ! {
        match &self.inner {
            Repr::Single { text, .. } => panic!("{}", text),
            Repr::SynError(error) => panic!("{}", error),
            Repr::Multi { diagnostics } => diagnostics[0].panic(),
        }
    }
}

impl From<Error> for Diagnostic {
    fn from(err: Error) -> Diagnostic {
        Diagnostic {
            inner: Repr::SynError(err),
        }
    }
}

pub fn extract_spans(node: &dyn ToTokens) -> Option<(Span, Span)> {
    let mut t = TokenStream::new();
    node.to_tokens(&mut t);
    let mut tokens = t.into_iter();
    let start = tokens.next().map(|t| t.span());
    let end = tokens.last().map(|t| t.span());
    start.map(|start| (start, end.unwrap_or(start)))
}

impl ToTokens for Diagnostic {
    fn to_tokens(&self, dst: &mut TokenStream) {
        match &self.inner {
            Repr::Single { text, span } => {
                let cs2 = (Span::call_site(), Span::call_site());
                let (start, end) = span.unwrap_or(cs2);
                dst.append(Ident::new("compile_error", start));
                dst.append(Punct::new('!', Spacing::Alone));
                let mut message = TokenStream::new();
                message.append(Literal::string(text));
                let mut group = Group::new(Delimiter::Brace, message);
                group.set_span(end);
                dst.append(group);
            }
            Repr::Multi { diagnostics } => {
                for diagnostic in diagnostics {
                    diagnostic.to_tokens(dst);
                }
            }
            Repr::SynError(err) => {
                err.to_compile_error().to_tokens(dst);
            }
        }
    }
}
