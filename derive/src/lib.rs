extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::{
    parse_macro_input, Attribute, AttributeArgs, Data, DeriveInput, Expr, Field, Fields, Ident,
    ImplItem, Item, Lit, Meta, NestedMeta,
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

#[proc_macro_derive(FromArgs, attributes(__inside_vm, pyarg))]
pub fn derive_from_args(input: TokenStream) -> TokenStream {
    let ast: DeriveInput = syn::parse(input).unwrap();

    let gen = impl_from_args(ast);
    gen.to_string().parse().unwrap()
}

/// The kind of the python parameter, this corresponds to the value of Parameter.kind
/// (https://docs.python.org/3/library/inspect.html#inspect.Parameter.kind)
enum ParameterKind {
    PositionalOnly,
    PositionalOrKeyword,
    KeywordOnly,
}

impl ParameterKind {
    fn from_ident(ident: &Ident) -> ParameterKind {
        if ident == "positional_only" {
            ParameterKind::PositionalOnly
        } else if ident == "positional_or_keyword" {
            ParameterKind::PositionalOrKeyword
        } else if ident == "keyword_only" {
            ParameterKind::KeywordOnly
        } else {
            panic!("Unrecognised attribute")
        }
    }
}

struct ArgAttribute {
    kind: ParameterKind,
    default: Option<Expr>,
    optional: bool,
}

impl ArgAttribute {
    fn from_attribute(attr: &Attribute) -> Option<ArgAttribute> {
        if !attr.path.is_ident("pyarg") {
            return None;
        }

        match attr.parse_meta().unwrap() {
            Meta::List(list) => {
                let mut iter = list.nested.iter();
                let first_arg = iter.next().expect("at least one argument in pyarg list");
                let kind = match first_arg {
                    NestedMeta::Meta(Meta::Word(ident)) => ParameterKind::from_ident(ident),
                    _ => panic!("Bad syntax for first pyarg attribute argument"),
                };

                let mut attribute = ArgAttribute {
                    kind,
                    default: None,
                    optional: false,
                };

                while let Some(arg) = iter.next() {
                    attribute.parse_argument(arg);
                }

                assert!(
                    attribute.default.is_none() || !attribute.optional,
                    "Can't set both a default value and optional"
                );

                Some(attribute)
            }
            _ => panic!("Bad syntax for pyarg attribute"),
        }
    }

    fn parse_argument(&mut self, arg: &NestedMeta) {
        match arg {
            NestedMeta::Meta(Meta::Word(ident)) => {
                if ident == "default" {
                    assert!(self.default.is_none(), "Default already set");
                    let expr = syn::parse_str::<Expr>("Default::default()").unwrap();
                    self.default = Some(expr);
                } else if ident == "optional" {
                    self.optional = true;
                } else {
                    panic!("Unrecognised pyarg attribute '{}'", ident);
                }
            }
            NestedMeta::Meta(Meta::NameValue(name_value)) => {
                if name_value.ident == "default" {
                    assert!(self.default.is_none(), "Default already set");

                    match name_value.lit {
                        Lit::Str(ref val) => {
                            let expr = val
                                .parse::<Expr>()
                                .expect("a valid expression for default argument");
                            self.default = Some(expr);
                        }
                        _ => panic!("Expected string value for default argument"),
                    }
                } else if name_value.ident == "optional" {
                    match name_value.lit {
                        Lit::Bool(ref val) => {
                            self.optional = val.value;
                        }
                        _ => panic!("Expected boolean value for optional argument"),
                    }
                } else {
                    panic!("Unrecognised pyarg attribute '{}'", name_value.ident);
                }
            }
            _ => panic!("Bad syntax for first pyarg attribute argument"),
        };
    }
}

fn generate_field(field: &Field) -> TokenStream2 {
    let mut pyarg_attrs = field
        .attrs
        .iter()
        .filter_map(ArgAttribute::from_attribute)
        .collect::<Vec<_>>();
    let attr = if pyarg_attrs.is_empty() {
        ArgAttribute {
            kind: ParameterKind::PositionalOrKeyword,
            default: None,
            optional: false,
        }
    } else if pyarg_attrs.len() == 1 {
        pyarg_attrs.remove(0)
    } else {
        panic!(
            "Multiple pyarg attributes on field '{}'",
            field.ident.as_ref().unwrap()
        );
    };

    let name = &field.ident;
    let middle = quote! {
        .map(|x| crate::pyobject::TryFromObject::try_from_object(vm, x)).transpose()?
    };
    let ending = if let Some(default) = attr.default {
        quote! {
            .unwrap_or_else(|| #default)
        }
    } else if attr.optional {
        quote! {
            .map(crate::function::OptionalArg::Present)
            .unwrap_or(crate::function::OptionalArg::Missing)
        }
    } else {
        let err = match attr.kind {
            ParameterKind::PositionalOnly | ParameterKind::PositionalOrKeyword => quote! {
                crate::function::ArgumentError::TooFewArgs
            },
            ParameterKind::KeywordOnly => quote! {
                crate::function::ArgumentError::RequiredKeywordArgument(tringify!(#name))
            },
        };
        quote! {
            .ok_or_else(|| #err)?
        }
    };

    match attr.kind {
        ParameterKind::PositionalOnly => {
            quote! {
                #name: args.take_positional()#middle#ending,
            }
        }
        ParameterKind::PositionalOrKeyword => {
            quote! {
                #name: args.take_positional_keyword(stringify!(#name))#middle#ending,
            }
        }
        ParameterKind::KeywordOnly => {
            quote! {
                #name: args.take_keyword(stringify!(#name))#middle#ending,
            }
        }
    }
}

fn impl_from_args(input: DeriveInput) -> TokenStream2 {
    let rp_path = rustpython_path_derive(&input);
    let fields = match input.data {
        Data::Struct(ref data) => {
            match data.fields {
                Fields::Named(ref fields) => fields.named.iter().map(generate_field),
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
