use proc_macro2::TokenStream;
use quote::quote;
use syn::DeriveInput;

pub(crate) fn impl_pypayload(input: DeriveInput) -> TokenStream {
    let ty = &input.ident;

    quote! {
        impl ::rustpython_vm::PyPayload for #ty {
            #[inline]
            fn class(ctx: &::rustpython_vm::vm::Context) -> ::rustpython_vm::builtins::PyTypeRef {
                <Self as ::rustpython_vm::class::PyClassImpl>::make_class(ctx)
            }
        }
    }
}
