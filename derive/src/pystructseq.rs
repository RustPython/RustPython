use super::Diagnostic;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Index};

pub(crate) fn impl_pystruct_sequence(
    input: DeriveInput,
) -> std::result::Result<TokenStream, Diagnostic> {
    let fields = if let syn::Data::Struct(ref struc) = input.data {
        &struc.fields
    } else {
        bail_span!(
            input,
            "#[pystruct_sequence] can only be on a struct declaration"
        )
    };

    let mut properties = Vec::new();
    let mut field_names = Vec::new();
    match fields {
        syn::Fields::Named(fields) => {
            for (i, field) in fields.named.iter().enumerate() {
                let idx = Index::from(i);
                let field_name = field.ident.as_ref().unwrap();
                let field_name_str = field_name.to_string();
                // TODO add doc to the generated property
                let property = quote! {
                    class.set_str_attr(
                        #field_name_str,
                        ctx.new_readonly_getset(
                            #field_name_str,
                            |zelf: &::rustpython_vm::obj::objtuple::PyTuple,
                             _vm: &::rustpython_vm::VirtualMachine| {
                                zelf.fast_getitem(#idx)
                            }
                       ),
                    );
                };
                properties.push(property);
                field_names.push(quote!(#field_name));
            }
        }
        _ => bail_span!(
            input,
            "#[pystruct_sequence] can only be on a struct with named fields"
        ),
    }

    let ty = &input.ident;
    let ret = quote! {
        impl ::rustpython_vm::pyobject::PyStructSequence for #ty {
            const FIELD_NAMES: &'static [&'static str] = &[#(stringify!(#field_names)),*];
            fn to_tuple(&self, vm: &::rustpython_vm::VirtualMachine,) -> ::rustpython_vm::obj::objtuple::PyTuple {
                let items = vec![#(::rustpython_vm::pyobject::IntoPyObject::into_pyobject(
                    ::std::clone::Clone::clone(&self.#field_names),
                    vm,
                )),*];
                items.into()
            }
        }
    };
    Ok(ret)
}
