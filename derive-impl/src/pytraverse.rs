use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, DeriveInput, Field, Meta, MetaList, NestedMeta, Result};

struct TraverseAttr {
    /// set to `true` if the attribute is `#[pytraverse(skip)]`
    skip: bool,
}

const ATTR_TRAVERSE: &str = "pytraverse";

/// get the `#[pytraverse(..)]` attribute from the struct
fn valid_get_traverse_attr_from_meta_list(list: &MetaList) -> Result<TraverseAttr> {
    let find_skip_and_only_skip = || {
        let len = list.nested.len();
        if len != 1 {
            return None;
        }
        let mut iter = list.nested.iter();
        // we have checked the length, so unwrap is safe
        let first_arg = iter.next().unwrap();
        let skip = match first_arg {
            NestedMeta::Meta(Meta::Path(path)) => match path.is_ident("skip") {
                true => true,
                false => return None,
            },
            _ => return None,
        };
        Some(skip)
    };
    let skip = find_skip_and_only_skip().ok_or_else(|| {
        err_span!(
            list,
            "only support attr is #[pytraverse(skip)], got arguments: {:?}",
            list.nested
        )
    })?;
    Ok(TraverseAttr { skip })
}

/// only accept `#[pytraverse(skip)]` for now
fn pytraverse_arg(attr: &Attribute) -> Option<Result<TraverseAttr>> {
    if !attr.path.is_ident(ATTR_TRAVERSE) {
        return None;
    }
    let ret = || {
        let parsed = attr.parse_meta()?;
        if let Meta::List(list) = parsed {
            valid_get_traverse_attr_from_meta_list(&list)
        } else {
            bail_span!(attr, "pytraverse must be a list, like #[pytraverse(skip)]")
        }
    };
    Some(ret())
}

fn field_to_traverse_code(field: &Field) -> Result<TokenStream> {
    let pytraverse_attrs = field
        .attrs
        .iter()
        .filter_map(pytraverse_arg)
        .collect::<std::result::Result<Vec<_>, _>>()?;
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
                syn::Fields::Named(ref mut fields) => {
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

    let ret = quote! {
        unsafe impl ::rustpython_vm::object::Traverse for #ty {
            fn traverse(&self, tracer_fn: &mut ::rustpython_vm::object::TraverseFn) {
                #trace_code
            }
        }
    };
    Ok(ret)
}
