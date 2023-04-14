use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Result};

/// also remove `#[notrace]` attr, and not trace corresponding field
fn gen_trace_code(item: &mut DeriveInput) -> Result<TokenStream> {
    match &mut item.data {
        syn::Data::Struct(s) => {
            let fields = &mut s.fields;
            if let syn::Fields::Named(ref mut fields) = fields {
                let res: Vec<TokenStream> = fields
                    .named
                    .iter_mut()
                    .map(|f| -> Result<TokenStream> {
                        let name = f.ident.as_ref().ok_or_else(|| {
                            syn::Error::new_spanned(
                                f.clone(),
                                "Field should have a name in non-tuple struct",
                            )
                        })?;
                        let mut do_trace = true;
                        f.attrs.retain(|attr| {
                            // remove #[notrace] and not trace this specifed field
                            if attr.path.segments.last().unwrap().ident == "notrace" {
                                do_trace = false;
                                false
                            } else {
                                true
                            }
                        });
                        if do_trace {
                            Ok(quote!(
                                ::rustpython_vm::object::gc::Traverse::traverse(&self.#name, tracer_fn);
                            ))
                        } else {
                            Ok(quote!())
                        }
                    })
                    .collect::<Result<_>>()?;
                let res = res.into_iter().collect::<TokenStream>();
                Ok(res)
            } else if let syn::Fields::Unnamed(fields) = fields {
                let res: TokenStream = (0..fields.unnamed.len())
                    .map(|i| {
                        let i = syn::Index::from(i);
                        quote!(
                            ::rustpython_vm::object::gc::Traverse::traverse(&self.#i, tracer_fn);
                        )
                    })
                    .collect();
                Ok(res)
            } else {
                Err(syn::Error::new_spanned(
                    fields,
                    "Only named fields are supported",
                ))
            }
        }
        _ => Err(syn::Error::new_spanned(item, "Only structs are supported")),
    }
}

pub(crate) fn impl_pytraverse(mut item: DeriveInput) -> Result<TokenStream> {
    let trace_code = gen_trace_code(&mut item)?;

    let ty = &item.ident;

    let ret = quote! {
        unsafe impl ::rustpython_vm::object::gc::Traverse for #ty {
            fn traverse(&self, tracer_fn: &mut ::rustpython_vm::object::gc::TraverseFn) {
                #trace_code
            }
        }
    };
    Ok(ret)
}
