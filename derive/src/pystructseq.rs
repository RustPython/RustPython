use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Result};

pub(crate) fn impl_pystruct_sequence(input: DeriveInput) -> Result<TokenStream> {
    let fields = if let syn::Data::Struct(ref struc) = input.data {
        &struc.fields
    } else {
        bail_span!(
            input,
            "#[pystruct_sequence] can only be on a struct declaration"
        )
    };

    let field_names: Vec<_> = match fields {
        syn::Fields::Named(fields) => fields
            .named
            .iter()
            .map(|field| field.ident.as_ref().unwrap())
            .collect(),
        _ => bail_span!(
            input,
            "#[pystruct_sequence] can only be on a struct with named fields"
        ),
    };

    let ty = &input.ident;
    let ret = quote! {
        impl ::rustpython_vm::PyStructSequence for #ty {
            const FIELD_NAMES: &'static [&'static str] = &[#(stringify!(#field_names)),*];
            fn into_tuple(self, vm: &::rustpython_vm::VirtualMachine) -> ::rustpython_vm::builtins::PyTuple {
                let items = vec![#(::rustpython_vm::IntoPyObject::into_pyobject(
                    self.#field_names,
                    vm,
                )),*];
                ::rustpython_vm::builtins::PyTuple::new_unchecked(items.into_boxed_slice())
            }
        }
        impl ::rustpython_vm::IntoPyObject for #ty {
            fn into_pyobject(self, vm: &::rustpython_vm::VirtualMachine) -> ::rustpython_vm::PyObjectRef {
                ::rustpython_vm::PyStructSequence::into_struct_sequence(self, vm).into_object()
            }
        }
    };
    Ok(ret)
}
