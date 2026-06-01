use proc_macro2::TokenStream;
use quote::quote;
use syn::DeriveInput;

pub(crate) fn impl_pypayload(input: DeriveInput) -> TokenStream {
    let ty = &input.ident;

    quote! {
        impl ::rustpython_vm::PyPayload for #ty {
            #[inline]
            fn class(_ctx: &::rustpython_vm::vm::Context) -> &'static rustpython_vm::Py<::rustpython_vm::builtins::PyType> {
                <Self as ::rustpython_vm::class::StaticType>::static_type()
            }
        }
    }
}
