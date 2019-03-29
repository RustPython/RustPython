extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::{
    parse_macro_input, AttributeArgs, Data, DeriveInput, Fields, Ident, ImplItem, Item, Lit, Meta,
    NestedMeta,
};

fn rustpython_path(inside_vm: bool) -> syn::Path {
    let path = if inside_vm {
        quote!(crate)
    } else {
        quote!(::rustpython_vm)
    };
    syn::parse2(path).unwrap()
}

/// Does the item have the #[__inside_vm] attribute on it, signifying that the derive target is
/// being derived from inside the `rustpython_vm` crate.
fn rustpython_path_derive(input: &DeriveInput) -> syn::Path {
    rustpython_path(
        input
            .attrs
            .iter()
            .any(|attr| attr.path.is_ident("__inside_vm")),
    )
}

fn rustpython_path_attr(attr: &AttributeArgs) -> syn::Path {
    rustpython_path(attr.iter().any(|meta| {
        if let syn::NestedMeta::Meta(meta) = meta {
            if let syn::Meta::Word(ident) = meta {
                ident == "__inside_vm"
            } else {
                false
            }
        } else {
            false
        }
    }))
}

#[proc_macro_derive(FromArgs, attributes(__inside_vm))]
pub fn derive_from_args(input: TokenStream) -> TokenStream {
    let ast: DeriveInput = syn::parse(input).unwrap();

    let gen = impl_from_args(ast);
    gen.to_string().parse().unwrap()
}

fn impl_from_args(input: DeriveInput) -> TokenStream2 {
    let rp_path = rustpython_path_derive(&input);
    let fields = match input.data {
        Data::Struct(ref data) => {
            match data.fields {
                Fields::Named(ref fields) => fields.named.iter().map(|field| {
                    let name = &field.ident;
                    quote! {
                        #name: crate::pyobject::TryFromObject::try_from_object(
                            vm,
                            args.take_keyword(stringify!(#name)).unwrap_or_else(|| vm.ctx.none())
                        )?,
                    }
                }),
                Fields::Unnamed(_) | Fields::Unit => unimplemented!(), // TODO: better error message
            }
        }
        Data::Enum(_) | Data::Union(_) => unimplemented!(), // TODO: better error message
    };

    let name = &input.ident;
    quote! {
        impl #rp_path::function::FromArgs for #name {
            fn from_args(
                vm: &crate::vm::VirtualMachine,
                args: &mut crate::function::PyFuncArgs
            ) -> Result<Self, crate::function::ArgumentError> {
                Ok(#name { #(#fields)* })
            }
        }
    }
}

#[proc_macro_attribute]
pub fn py_class(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_macro_input!(attr as AttributeArgs);
    let item = parse_macro_input!(item as Item);
    impl_py_class(attr, item).into()
}

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

fn item_impl_to_methods<'a>(imp: &'a syn::ItemImpl) -> impl Iterator<Item = Method> + 'a {
    imp.items.iter().filter_map(|item| {
        if let ImplItem::Method(meth) = item {
            let mut py_name = None;
            let mut kind = MethodKind::Method;
            let metas_iter = meth
                .attrs
                .iter()
                .filter_map(|attr| {
                    if attr.path.is_ident("py_class") {
                        let meta = attr.parse_meta().expect("Invalid attribute");
                        if let Meta::List(list) = meta {
                            Some(list)
                        } else {
                            panic!(
                                "#[py_class] attribute on a method should be a list, like \
                                 #[py_class(...)]"
                            )
                        }
                    } else {
                        None
                    }
                })
                .flat_map(|attr| attr.nested);
            for meta in metas_iter {
                if let NestedMeta::Meta(meta) = meta {
                    match meta {
                        Meta::NameValue(name_value) => {
                            if name_value.ident == "name" {
                                if let Lit::Str(s) = &name_value.lit {
                                    py_name = Some(s.value());
                                } else {
                                    panic!("#[py_class(name = ...)] must be a string");
                                }
                            }
                        }
                        Meta::Word(ident) => match ident.to_string().as_str() {
                            "property" => kind = MethodKind::Property,
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }
            let py_name = py_name.unwrap_or_else(|| meth.sig.ident.to_string());
            Some(Method {
                fn_name: meth.sig.ident.clone(),
                py_name,
                kind,
            })
        } else {
            None
        }
    })
}

fn impl_py_class(attr: AttributeArgs, item: Item) -> TokenStream2 {
    let imp = if let Item::Impl(imp) = item {
        imp
    } else {
        return quote!(#item);
    };
    let rp_path = rustpython_path_attr(&attr);
    let mut class_name = None;
    let mut doc = None;
    for attr in attr {
        if let NestedMeta::Meta(meta) = attr {
            if let Meta::NameValue(name_value) = meta {
                if name_value.ident == "name" {
                    if let Lit::Str(s) = name_value.lit {
                        class_name = Some(s.value());
                    } else {
                        panic!("#[py_class(name = ...)] must be a string");
                    }
                } else if name_value.ident == "doc" {
                    if let Lit::Str(s) = name_value.lit {
                        doc = Some(s.value());
                    } else {
                        panic!("#[py_class(name = ...)] must be a string");
                    }
                }
            }
        }
    }
    let class_name = class_name.expect("#[py_class] must have a name");
    let doc = match doc {
        Some(doc) => quote!(Some(#doc)),
        None => quote!(None),
    };
    let ty = &imp.self_ty;
    let methods = item_impl_to_methods(&imp).map(
        |Method {
             py_name,
             fn_name,
             kind,
         }| {
            let constructor_fn = kind.to_ctx_constructor_fn();
            quote! {
                ctx.set_attr(class, #py_name, ctx.#constructor_fn(#ty::#fn_name));
            }
        },
    );

    quote! {
        #imp
        impl #rp_path::pyobject::IntoPyClass for #ty {
            const NAME: &'static str = #class_name;
            const DOC: Option<&'static str> = #doc;
            fn _extend_class(
                ctx: &#rp_path::pyobject::PyContext,
                class: &#rp_path::obj::objtype::PyClassRef,
            ) {
                #(#methods)*
            }
        }
    }
}
