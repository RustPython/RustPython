use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, DeriveInput, Field, Result};

struct TraverseAttr {
    /// set to `true` if the attribute is `#[pytraverse(skip)]`
    skip: bool,
}

const ATTR_TRAVERSE: &str = "pytraverse";

/// only accept `#[pytraverse(skip)]` for now
fn pytraverse_arg(attr: &Attribute) -> Option<Result<TraverseAttr>> {
    if !attr.path().is_ident(ATTR_TRAVERSE) {
        return None;
    }
    let ret = || {
        let mut skip = false;
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") {
                if skip {
                    return Err(meta.error("already specified skip"));
                }
                skip = true;
            } else {
                return Err(meta.error("unknown attr"));
            }
            Ok(())
        })?;
        Ok(TraverseAttr { skip })
    };
    Some(ret())
}

fn field_to_traverse_code(field: &Field) -> Result<TokenStream> {
    let pytraverse_attrs = field
        .attrs
        .iter()
        .filter_map(pytraverse_arg)
        .collect::<core::result::Result<Vec<_>, _>>()?;
    let do_trace = if pytraverse_attrs.len() > 1 {
        bail_span!(
            field,
            "found multiple #[pytraverse] attributes on the same field, expect at most one"
        )
    } else if pytraverse_attrs.is_empty() {
        // default to always traverse every field
        true
    } else {
        !pytraverse_attrs[0].skip
    };
    let name = field.ident.as_ref().ok_or_else(|| {
        syn::Error::new_spanned(
            field.clone(),
            "Field should have a name in non-tuple struct",
        )
    })?;
    if do_trace {
        Ok(quote!(
            ::rustpython_vm::object::Traverse::traverse(&self.#name, tracer_fn);
        ))
    } else {
        Ok(quote!())
    }
}

/// not trace corresponding field
fn gen_trace_code(item: &mut DeriveInput) -> Result<TokenStream> {
    match &mut item.data {
        syn::Data::Struct(s) => {
            let fields = &mut s.fields;
            match fields {
                syn::Fields::Named(fields) => {
                    let res: Vec<TokenStream> = fields
                        .named
                        .iter_mut()
                        .map(|f| -> Result<TokenStream> { field_to_traverse_code(f) })
                        .collect::<Result<_>>()?;
                    let res = res.into_iter().collect::<TokenStream>();
                    Ok(res)
                }
                syn::Fields::Unnamed(fields) => {
                    let res: TokenStream = (0..fields.unnamed.len())
                        .map(|i| {
                            let i = syn::Index::from(i);
                            quote!(
                                ::rustpython_vm::object::Traverse::traverse(&self.#i, tracer_fn);
                            )
                        })
                        .collect();
                    Ok(res)
                }
                _ => Err(syn::Error::new_spanned(
                    fields,
                    "Only named and unnamed fields are supported",
                )),
            }
        }
        _ => Err(syn::Error::new_spanned(item, "Only structs are supported")),
    }
}

pub(crate) fn impl_pytraverse(mut item: DeriveInput) -> Result<TokenStream> {
    let trace_code = gen_trace_code(&mut item)?;

    let ty = &item.ident;

    // Add Traverse bound to all type parameters
    for param in &mut item.generics.params {
        if let syn::GenericParam::Type(type_param) = param {
            type_param
                .bounds
                .push(syn::parse_quote!(::rustpython_vm::object::Traverse));
        }
    }

    let (impl_generics, ty_generics, where_clause) = item.generics.split_for_impl();

    let ret = quote! {
        unsafe impl #impl_generics ::rustpython_vm::object::Traverse for #ty #ty_generics #where_clause {
            fn traverse(&self, tracer_fn: &mut ::rustpython_vm::object::TraverseFn) {
                #trace_code
            }
        }
    };
    Ok(ret)
}
