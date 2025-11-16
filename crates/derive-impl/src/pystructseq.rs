use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Ident, Result};
use syn_ext::ext::{AttributeExt, GetIdent};
use syn_ext::types::Meta;

// returning a pair of not-skipped and skipped field names
fn field_names(input: &mut DeriveInput) -> Result<(Vec<Ident>, Vec<Ident>)> {
    let syn::Data::Struct(struc) = &mut input.data else {
        bail_span!(
            input,
            "#[pystruct_sequence] can only be on a struct declaration"
        )
    };

    let syn::Fields::Named(fields) = &mut struc.fields else {
        bail_span!(
            input,
            "#[pystruct_sequence] can only be on a struct with named fields"
        );
    };

    let mut not_skipped = Vec::with_capacity(fields.named.len());
    let mut skipped = Vec::with_capacity(fields.named.len());
    for field in &mut fields.named {
        let mut skip = false;
        // Collect all attributes with pystruct and their indices
        let mut attrs_to_remove = Vec::new();

        for (i, attr) in field.attrs.iter().enumerate() {
            if !attr.path().is_ident("pystruct") {
                continue;
            }

            let Ok(meta) = attr.parse_meta() else {
                continue;
            };

            let Meta::List(l) = meta else {
                bail_span!(input, "Only #[pystruct(...)] form is allowed");
            };

            let idents: Vec<_> = l
                .nested
                .iter()
                .filter_map(|n| n.get_ident())
                .cloned()
                .collect();

            // Follow #[serde(skip)] convention.
            // Consider to add skip_serializing and skip_deserializing if required.
            for ident in idents {
                match ident.to_string().as_str() {
                    "skip" => {
                        skip = true;
                    }
                    _ => {
                        bail_span!(ident, "Unknown item for #[pystruct(...)]")
                    }
                }
            }

            attrs_to_remove.push(i);
        }

        // Remove attributes in reverse order to maintain valid indices
        attrs_to_remove.sort_unstable_by(|a, b| b.cmp(a)); // Sort in descending order
        for index in attrs_to_remove {
            field.attrs.remove(index);
        }
        let ident = field.ident.clone().unwrap();
        if skip {
            skipped.push(ident.clone());
        } else {
            not_skipped.push(ident.clone());
        }
    }

    Ok((not_skipped, skipped))
}

pub(crate) fn impl_pystruct_sequence(mut input: DeriveInput) -> Result<TokenStream> {
    let (not_skipped_fields, skipped_fields) = field_names(&mut input)?;
    let ty = &input.ident;
    let ret = quote! {
        impl ::rustpython_vm::types::PyStructSequence for #ty {
            const REQUIRED_FIELD_NAMES: &'static [&'static str] = &[#(stringify!(#not_skipped_fields),)*];
            const OPTIONAL_FIELD_NAMES: &'static [&'static str] = &[#(stringify!(#skipped_fields),)*];
            fn into_tuple(self, vm: &::rustpython_vm::VirtualMachine) -> ::rustpython_vm::builtins::PyTuple {
                let items = vec![
                    #(::rustpython_vm::convert::ToPyObject::to_pyobject(
                        self.#not_skipped_fields,
                        vm,
                    ),)*
                ];
                ::rustpython_vm::builtins::PyTuple::new_unchecked(items.into_boxed_slice())
            }
        }
        impl ::rustpython_vm::convert::ToPyObject for #ty {
            fn to_pyobject(self, vm: &::rustpython_vm::VirtualMachine) -> ::rustpython_vm::PyObjectRef {
                ::rustpython_vm::types::PyStructSequence::into_struct_sequence(self, vm).into()
            }
        }
    };
    Ok(ret)
}

pub(crate) fn impl_pystruct_sequence_try_from_object(
    mut input: DeriveInput,
) -> Result<TokenStream> {
    let (not_skipped_fields, skipped_fields) = field_names(&mut input)?;
    let ty = &input.ident;
    let ret = quote! {
        impl ::rustpython_vm::TryFromObject for #ty {
            fn try_from_object(vm: &::rustpython_vm::VirtualMachine, seq: ::rustpython_vm::PyObjectRef) -> ::rustpython_vm::PyResult<Self> {
                let seq = Self::try_elements_from(seq, vm)?;
                let mut iter = seq.into_iter();
                Ok(Self {
                    #(#not_skipped_fields: iter.next().unwrap().clone().try_into_value(vm)?,)*
                    #(#skipped_fields: match iter.next() {
                        Some(v) => v.clone().try_into_value(vm)?,
                        None => vm.ctx.none(),
                    },)*
                })
            }
        }
    };
    Ok(ret)
}
