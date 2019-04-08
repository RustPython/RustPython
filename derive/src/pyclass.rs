use super::rustpython_path_attr;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::{Attribute, AttributeArgs, Ident, ImplItem, Item, Lit, Meta, MethodSig, NestedMeta};

enum MethodKind {
    Method,
    Property,
}

impl MethodKind {
    fn to_ctx_constructor_fn(&self) -> Ident {
        let f = match self {
            MethodKind::Method => "new_rustfunc",
            MethodKind::Property => "new_property",
        };
        Ident::new(f, Span::call_site())
    }
}

struct Method {
    fn_name: Ident,
    py_name: String,
    kind: MethodKind,
}

impl Method {
    fn from_syn(attrs: &mut Vec<Attribute>, sig: &MethodSig) -> Option<Method> {
        let mut py_name = None;
        let mut kind = MethodKind::Method;
        let mut pymethod_to_remove = Vec::new();
        let metas = attrs
            .iter()
            .enumerate()
            .filter_map(|(i, attr)| {
                if attr.path.is_ident("pymethod") {
                    let meta = attr.parse_meta().expect("Invalid attribute");
                    // remove #[pymethod] because there's no actual proc macro
                    // implementation for it
                    pymethod_to_remove.push(i);
                    match meta {
                        Meta::List(list) => Some(list),
                        Meta::Word(_) => None,
                        Meta::NameValue(_) => panic!(
                            "#[pymethod = ...] attribute on a method should be a list, like \
                             #[pymethod(...)]"
                        ),
                    }
                } else {
                    None
                }
            })
            .flat_map(|attr| attr.nested);
        for meta in metas {
            if let NestedMeta::Meta(meta) = meta {
                match meta {
                    Meta::NameValue(name_value) => {
                        if name_value.ident == "name" {
                            if let Lit::Str(s) = &name_value.lit {
                                py_name = Some(s.value());
                            } else {
                                panic!("#[pymethod(name = ...)] must be a string");
                            }
                        }
                    }
                    Meta::Word(ident) => {
                        if ident == "property" {
                            kind = MethodKind::Property
                        }
                    }
                    _ => {}
                }
            }
        }
        // if there are no #[pymethods]s, then it's not a method, so exclude it from
        // the final result
        if pymethod_to_remove.is_empty() {
            return None;
        }
        for i in pymethod_to_remove {
            attrs.remove(i);
        }
        let py_name = py_name.unwrap_or_else(|| sig.ident.to_string());
        Some(Method {
            fn_name: sig.ident.clone(),
            py_name,
            kind,
        })
    }
}

pub fn impl_pyimpl(attr: AttributeArgs, item: Item) -> TokenStream2 {
    let mut imp = if let Item::Impl(imp) = item {
        imp
    } else {
        return quote!(#item);
    };

    let rp_path = rustpython_path_attr(&attr);

    let methods = imp
        .items
        .iter_mut()
        .filter_map(|item| {
            if let ImplItem::Method(meth) = item {
                Method::from_syn(&mut meth.attrs, &meth.sig)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let ty = &imp.self_ty;
    let methods = methods.iter().map(
        |Method {
             py_name,
             fn_name,
             kind,
         }| {
            let constructor_fn = kind.to_ctx_constructor_fn();
            quote! {
                class.set_str_attr(#py_name, ctx.#constructor_fn(Self::#fn_name));
            }
        },
    );

    quote! {
        #imp
        impl #rp_path::pyobject::PyClassImpl for #ty {
            fn impl_extend_class(
                ctx: &#rp_path::pyobject::PyContext,
                class: &#rp_path::obj::objtype::PyClassRef,
            ) {
                #(#methods)*
            }
        }
    }
}

pub fn impl_pyclass(attr: AttributeArgs, item: Item) -> TokenStream2 {
    let (item, ident, attrs) = match item {
        Item::Struct(struc) => (quote!(#struc), struc.ident, struc.attrs),
        Item::Enum(enu) => (quote!(#enu), enu.ident, enu.attrs),
        _ => panic!("#[pyclass] can only be on a struct or enum declaration"),
    };

    let rp_path = rustpython_path_attr(&attr);

    let mut class_name = None;
    for attr in attr {
        if let NestedMeta::Meta(meta) = attr {
            if let Meta::NameValue(name_value) = meta {
                if name_value.ident == "name" {
                    if let Lit::Str(s) = name_value.lit {
                        class_name = Some(s.value());
                    } else {
                        panic!("#[pyclass(name = ...)] must be a string");
                    }
                }
            }
        }
    }
    let class_name = class_name.unwrap_or_else(|| ident.to_string());

    let mut doc: Option<Vec<String>> = None;
    for attr in attrs.iter() {
        if attr.path.is_ident("doc") {
            let meta = attr.parse_meta().expect("expected doc attr to be a meta");
            if let Meta::NameValue(name_value) = meta {
                if let Lit::Str(s) = name_value.lit {
                    let val = s.value().trim().to_string();
                    match doc {
                        Some(ref mut doc) => doc.push(val),
                        None => doc = Some(vec![val]),
                    }
                } else {
                    panic!("expected #[doc = ...] to be a string")
                }
            }
        }
    }
    let doc = match doc {
        Some(doc) => {
            let doc = doc.join("\n");
            quote!(Some(#doc))
        }
        None => quote!(None),
    };

    quote! {
        #item
        impl #rp_path::pyobject::PyClassDef for #ident {
            const NAME: &'static str = #class_name;
            const DOC: Option<&'static str> = #doc;
        }
    }
}
