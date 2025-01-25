use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Ident, Result};
use syn_ext::ext::GetIdent;

struct Field {
    ident: Ident,
    skip: bool,
}

fn collect_fields(input: &mut DeriveInput) -> Result<Vec<Field>> {
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

    let mut fields_info = Vec::with_capacity(fields.named.len());
    for field in &mut fields.named {
        let mut removed_indices = Vec::new();
        let mut skip = false;
        for (i, attr) in field.attrs.iter().enumerate().rev() {
            if !attr.path.is_ident("pystruct") {
                continue;
            }
            removed_indices.push(i);
            let Ok(meta) = attr.parse_meta() else {
                continue;
            };
            let syn::Meta::List(l) = meta else {
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
        }
        eprintln!("removed indices: {:?}", removed_indices);
        for index in removed_indices {
            field.attrs.remove(index);
        }
        let ident = field.ident.clone().unwrap();
        fields_info.push(Field { ident, skip });
    }

    Ok(fields_info)
}

pub(crate) fn impl_pystruct_sequence(mut input: DeriveInput) -> Result<TokenStream> {
    let fields = collect_fields(&mut input)?;
    let field_names: Vec<_> = fields.iter().map(|f| f.ident.clone()).collect();
    let ty = &input.ident;
    let ret = quote! {
        impl ::rustpython_vm::types::PyStructSequence for #ty {
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
                ::rustpython_vm::types::PyStructSequence::into_struct_sequence(self, vm).into()
            }
        }
    };
    Ok(ret)
}

pub(crate) fn impl_pystruct_sequence_try_from_object(
    mut input: DeriveInput,
) -> Result<TokenStream> {
    let fields = collect_fields(&mut input)?;
    let ty = &input.ident;
    let field_exprs: Vec<_> = fields.into_iter().map(|f| {
        let name = f.ident.clone();
        if f.skip {
            // FIXME: expected to be customizable
            quote! { #name: vm.ctx.none().clone().into() }
        } else {
            quote! { #name: iter.next().unwrap().clone().try_into_value(vm)? }
        }
    }).collect();
    let ret = quote! {
        impl ::rustpython_vm::TryFromObject for #ty {
            fn try_from_object(vm: &::rustpython_vm::VirtualMachine, seq: ::rustpython_vm::PyObjectRef) -> ::rustpython_vm::PyResult<Self> {
                const LEN: usize = #ty::FIELD_NAMES.len();
                let seq = Self::try_elements_from::<LEN>(seq, vm)?;
                // TODO: this is possible to be written without iterator
                let mut iter = seq.into_iter();
                Ok(Self {#(
                    #field_exprs
                ),*})
            }
        }
    };
    Ok(ret)
}
