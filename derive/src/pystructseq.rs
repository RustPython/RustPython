use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Ident, Result};

fn field_names(input: &DeriveInput) -> Result<Vec<&Ident>> {
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

    Ok(field_names)
}

pub(crate) fn impl_pystruct_sequence(input: DeriveInput) -> Result<TokenStream> {
    let field_names = field_names(&input)?;
    let ty = &input.ident;
    let ret = quote! {
        impl ::rustpython_vm::PyStructSequence for #ty {
            const FIELD_LEN: usize = [#(
                stringify!(#field_names)
            ),*].len();
            const FIELD_NAMES: &'static [&'static str] = &[#(stringify!(#field_names)),*];
            fn into_tuple(self, vm: &::rustpython_vm::VirtualMachine) -> ::rustpython_vm::builtins::PyTuple {
                let items = vec![#(::rustpython_vm::convert::ToPyObject::to_pyobject(
                    self.#field_names,
                    vm,
                )),*];
                ::rustpython_vm::builtins::PyTuple::new_unchecked(items.into_boxed_slice())
            }
        }
        impl ::rustpython_vm::convert::ToPyObject for #ty {
            fn to_pyobject(self, vm: &::rustpython_vm::VirtualMachine) -> ::rustpython_vm::PyObjectRef {
                ::rustpython_vm::PyStructSequence::into_struct_sequence(self, vm).into()
            }
        }
    };
    Ok(ret)
}

pub(crate) fn impl_pystruct_sequence_try_from_object(input: DeriveInput) -> Result<TokenStream> {
    let field_names = field_names(&input)?;
    let ty = &input.ident;
    let ret = quote! {
        impl ::rustpython_vm::TryFromObject for #ty {
            fn try_from_object(vm: &::rustpython_vm::VirtualMachine, seq: ::rustpython_vm::PyObjectRef) -> ::rustpython_vm::PyResult<Self> {
                let seq = Self::try_elements_from(seq, vm)?;
                let mut iter = seq.into_iter();
                Ok(Self {#(
                    #field_names: iter.next().unwrap().clone().try_into_value(vm)?
                ),*})
            }
        }
    };
    Ok(ret)
}
