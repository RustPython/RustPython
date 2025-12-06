use crate::util::{ItemMeta, ItemMetaInner};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, Ident, Item, Result};
use syn_ext::ext::{AttributeExt, GetIdent};
use syn_ext::types::{Meta, PunctuatedNestedMeta};

// #[pystruct_sequence_data] - For Data structs

/// Field kind for struct sequence
#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    /// Named visible field (has getter, shown in repr)
    Named,
    /// Unnamed visible field (index-only, no getter)
    Unnamed,
    /// Hidden/skipped field (stored in tuple, but hidden from repr/len/index)
    Skipped,
}

/// Parsed field with its kind
struct ParsedField {
    ident: Ident,
    kind: FieldKind,
}

/// Parsed field info from struct
struct FieldInfo {
    /// All fields in order with their kinds
    fields: Vec<ParsedField>,
}

impl FieldInfo {
    fn named_fields(&self) -> Vec<Ident> {
        self.fields
            .iter()
            .filter(|f| f.kind == FieldKind::Named)
            .map(|f| f.ident.clone())
            .collect()
    }

    fn visible_fields(&self) -> Vec<Ident> {
        self.fields
            .iter()
            .filter(|f| f.kind != FieldKind::Skipped)
            .map(|f| f.ident.clone())
            .collect()
    }

    fn skipped_fields(&self) -> Vec<Ident> {
        self.fields
            .iter()
            .filter(|f| f.kind == FieldKind::Skipped)
            .map(|f| f.ident.clone())
            .collect()
    }

    fn n_unnamed_fields(&self) -> usize {
        self.fields
            .iter()
            .filter(|f| f.kind == FieldKind::Unnamed)
            .count()
    }
}

/// Parse field info from struct
fn parse_fields(input: &mut DeriveInput) -> Result<FieldInfo> {
    let syn::Data::Struct(struc) = &mut input.data else {
        bail_span!(input, "#[pystruct_sequence_data] can only be on a struct")
    };

    let syn::Fields::Named(fields) = &mut struc.fields else {
        bail_span!(
            input,
            "#[pystruct_sequence_data] can only be on a struct with named fields"
        );
    };

    let mut parsed_fields = Vec::with_capacity(fields.named.len());

    for field in &mut fields.named {
        let mut skip = false;
        let mut unnamed = false;
        let mut attrs_to_remove = Vec::new();

        for (i, attr) in field.attrs.iter().enumerate() {
            if !attr.path().is_ident("pystruct_sequence") {
                continue;
            }

            let Ok(meta) = attr.parse_meta() else {
                continue;
            };

            let Meta::List(l) = meta else {
                bail_span!(input, "Only #[pystruct_sequence(...)] form is allowed");
            };

            let idents: Vec<_> = l
                .nested
                .iter()
                .filter_map(|n| n.get_ident())
                .cloned()
                .collect();

            for ident in idents {
                match ident.to_string().as_str() {
                    "skip" => {
                        skip = true;
                    }
                    "unnamed" => {
                        unnamed = true;
                    }
                    _ => {
                        bail_span!(ident, "Unknown item for #[pystruct_sequence(...)]")
                    }
                }
            }

            attrs_to_remove.push(i);
        }

        // Remove attributes in reverse order
        attrs_to_remove.sort_unstable_by(|a, b| b.cmp(a));
        for index in attrs_to_remove {
            field.attrs.remove(index);
        }

        let ident = field.ident.clone().unwrap();
        let kind = if skip {
            FieldKind::Skipped
        } else if unnamed {
            FieldKind::Unnamed
        } else {
            FieldKind::Named
        };

        parsed_fields.push(ParsedField { ident, kind });
    }

    Ok(FieldInfo {
        fields: parsed_fields,
    })
}

/// Check if `try_from_object` is present in attribute arguments
fn has_try_from_object(attr: &PunctuatedNestedMeta) -> bool {
    attr.iter().any(|nested| {
        nested
            .get_ident()
            .is_some_and(|ident| ident == "try_from_object")
    })
}

/// Attribute macro for Data structs: #[pystruct_sequence_data(...)]
///
/// Generates:
/// - `REQUIRED_FIELD_NAMES` constant (named visible fields)
/// - `OPTIONAL_FIELD_NAMES` constant (hidden/skipped fields)
/// - `UNNAMED_FIELDS_LEN` constant
/// - `into_tuple()` method
/// - Field index constants (e.g., `TM_YEAR_INDEX`)
///
/// Options:
/// - `try_from_object`: Generate `try_from_elements()` method and `TryFromObject` impl
pub(crate) fn impl_pystruct_sequence_data(
    attr: PunctuatedNestedMeta,
    item: Item,
) -> Result<TokenStream> {
    let Item::Struct(item_struct) = item else {
        bail_span!(
            item,
            "#[pystruct_sequence_data] can only be applied to structs"
        );
    };

    let try_from_object = has_try_from_object(&attr);
    let mut input: DeriveInput = DeriveInput {
        attrs: item_struct.attrs.clone(),
        vis: item_struct.vis.clone(),
        ident: item_struct.ident.clone(),
        generics: item_struct.generics.clone(),
        data: syn::Data::Struct(syn::DataStruct {
            struct_token: item_struct.struct_token,
            fields: item_struct.fields.clone(),
            semi_token: item_struct.semi_token,
        }),
    };
    let field_info = parse_fields(&mut input)?;
    let data_ident = &input.ident;

    let named_fields = field_info.named_fields();
    let visible_fields = field_info.visible_fields();
    let skipped_fields = field_info.skipped_fields();
    let n_unnamed_fields = field_info.n_unnamed_fields();

    // Generate field index constants for visible fields
    let field_indices: Vec<_> = visible_fields
        .iter()
        .enumerate()
        .map(|(i, field)| {
            let const_name = format_ident!("{}_INDEX", field.to_string().to_uppercase());
            quote! {
                pub const #const_name: usize = #i;
            }
        })
        .collect();

    // Generate TryFromObject impl only when try_from_object=true
    let try_from_object_impl = if try_from_object {
        let n_required = visible_fields.len();
        quote! {
            impl ::rustpython_vm::TryFromObject for #data_ident {
                fn try_from_object(
                    vm: &::rustpython_vm::VirtualMachine,
                    obj: ::rustpython_vm::PyObjectRef,
                ) -> ::rustpython_vm::PyResult<Self> {
                    let seq: Vec<::rustpython_vm::PyObjectRef> = obj.try_into_value(vm)?;
                    if seq.len() < #n_required {
                        return Err(vm.new_type_error(format!(
                            "{} requires at least {} elements",
                            stringify!(#data_ident),
                            #n_required
                        )));
                    }
                    <Self as ::rustpython_vm::types::PyStructSequenceData>::try_from_elements(seq, vm)
                }
            }
        }
    } else {
        quote! {}
    };

    // Generate try_from_elements trait override only when try_from_object=true
    let try_from_elements_trait_override = if try_from_object {
        quote! {
            fn try_from_elements(
                elements: Vec<::rustpython_vm::PyObjectRef>,
                vm: &::rustpython_vm::VirtualMachine,
            ) -> ::rustpython_vm::PyResult<Self> {
                let mut iter = elements.into_iter();
                Ok(Self {
                    #(#visible_fields: iter.next().unwrap().clone().try_into_value(vm)?,)*
                    #(#skipped_fields: match iter.next() {
                        Some(v) => v.clone().try_into_value(vm)?,
                        None => vm.ctx.none(),
                    },)*
                })
            }
        }
    } else {
        quote! {}
    };

    let output = quote! {
        impl #data_ident {
            #(#field_indices)*
        }

        // PyStructSequenceData trait impl
        impl ::rustpython_vm::types::PyStructSequenceData for #data_ident {
            const REQUIRED_FIELD_NAMES: &'static [&'static str] = &[#(stringify!(#named_fields),)*];
            const OPTIONAL_FIELD_NAMES: &'static [&'static str] = &[#(stringify!(#skipped_fields),)*];
            const UNNAMED_FIELDS_LEN: usize = #n_unnamed_fields;

            fn into_tuple(self, vm: &::rustpython_vm::VirtualMachine) -> ::rustpython_vm::builtins::PyTuple {
                let items = vec![
                    #(::rustpython_vm::convert::ToPyObject::to_pyobject(
                        self.#visible_fields,
                        vm,
                    ),)*
                    #(::rustpython_vm::convert::ToPyObject::to_pyobject(
                        self.#skipped_fields,
                        vm,
                    ),)*
                ];
                ::rustpython_vm::builtins::PyTuple::new_unchecked(items.into_boxed_slice())
            }

            #try_from_elements_trait_override
        }

        #try_from_object_impl
    };

    // For attribute macro, we need to output the original struct as well
    // But first, strip #[pystruct_sequence] attributes from fields
    let mut clean_struct = item_struct.clone();
    if let syn::Fields::Named(ref mut fields) = clean_struct.fields {
        for field in &mut fields.named {
            field
                .attrs
                .retain(|attr| !attr.path().is_ident("pystruct_sequence"));
        }
    }

    Ok(quote! {
        #clean_struct
        #output
    })
}

// #[pystruct_sequence(...)] - For Python type structs

/// Meta parser for #[pystruct_sequence(...)]
pub(crate) struct PyStructSequenceMeta {
    inner: ItemMetaInner,
}

impl ItemMeta for PyStructSequenceMeta {
    const ALLOWED_NAMES: &'static [&'static str] = &["name", "module", "data", "no_attr"];

    fn from_inner(inner: ItemMetaInner) -> Self {
        Self { inner }
    }
    fn inner(&self) -> &ItemMetaInner {
        &self.inner
    }
}

impl PyStructSequenceMeta {
    pub fn class_name(&self) -> Result<Option<String>> {
        const KEY: &str = "name";
        let inner = self.inner();
        if let Some((_, meta)) = inner.meta_map.get(KEY) {
            if let Meta::NameValue(syn::MetaNameValue {
                value:
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(lit),
                        ..
                    }),
                ..
            }) = meta
            {
                return Ok(Some(lit.value()));
            }
            bail_span!(
                inner.meta_ident,
                "#[pystruct_sequence({KEY}=value)] expects a string value"
            )
        } else {
            Ok(None)
        }
    }

    pub fn module(&self) -> Result<Option<String>> {
        const KEY: &str = "module";
        let inner = self.inner();
        if let Some((_, meta)) = inner.meta_map.get(KEY) {
            if let Meta::NameValue(syn::MetaNameValue {
                value:
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(lit),
                        ..
                    }),
                ..
            }) = meta
            {
                return Ok(Some(lit.value()));
            }
            bail_span!(
                inner.meta_ident,
                "#[pystruct_sequence({KEY}=value)] expects a string value"
            )
        } else {
            Ok(None)
        }
    }

    fn data_type(&self) -> Result<Ident> {
        const KEY: &str = "data";
        let inner = self.inner();
        if let Some((_, meta)) = inner.meta_map.get(KEY) {
            if let Meta::NameValue(syn::MetaNameValue {
                value:
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(lit),
                        ..
                    }),
                ..
            }) = meta
            {
                return Ok(format_ident!("{}", lit.value()));
            }
            bail_span!(
                inner.meta_ident,
                "#[pystruct_sequence({KEY}=value)] expects a string value"
            )
        } else {
            bail_span!(
                inner.meta_ident,
                "#[pystruct_sequence] requires data parameter (e.g., data = \"DataStructName\")"
            )
        }
    }

    pub fn no_attr(&self) -> Result<bool> {
        self.inner()._bool("no_attr")
    }
}

/// Attribute macro for struct sequences.
///
/// Usage:
/// ```ignore
/// #[pystruct_sequence_data]
/// struct StructTimeData { ... }
///
/// #[pystruct_sequence(name = "struct_time", module = "time", data = "StructTimeData")]
/// struct PyStructTime;
/// ```
pub(crate) fn impl_pystruct_sequence(
    attr: PunctuatedNestedMeta,
    item: Item,
) -> Result<TokenStream> {
    let Item::Struct(struct_item) = item else {
        bail_span!(item, "#[pystruct_sequence] can only be applied to a struct");
    };

    let ident = struct_item.ident.clone();
    let fake_ident = Ident::new("pystruct_sequence", ident.span());
    let meta = PyStructSequenceMeta::from_nested(ident, fake_ident, attr.into_iter())?;

    let pytype_ident = struct_item.ident.clone();
    let pytype_vis = struct_item.vis.clone();
    let data_ident = meta.data_type()?;

    let class_name = meta.class_name()?.ok_or_else(|| {
        syn::Error::new_spanned(
            &struct_item.ident,
            "#[pystruct_sequence] requires name parameter",
        )
    })?;
    let module_name = meta.module()?;

    // Module name handling
    let module_name_tokens = match &module_name {
        Some(m) => quote!(Some(#m)),
        None => quote!(None),
    };

    let module_class_name = if let Some(ref m) = module_name {
        format!("{}.{}", m, class_name)
    } else {
        class_name.clone()
    };

    let output = quote! {
        // The Python type struct - newtype wrapping PyTuple
        #[repr(transparent)]
        #pytype_vis struct #pytype_ident(pub ::rustpython_vm::builtins::PyTuple);

        // PyClassDef for Python type
        impl ::rustpython_vm::class::PyClassDef for #pytype_ident {
            const NAME: &'static str = #class_name;
            const MODULE_NAME: Option<&'static str> = #module_name_tokens;
            const TP_NAME: &'static str = #module_class_name;
            const DOC: Option<&'static str> = None;
            const BASICSIZE: usize = 0;
            const UNHASHABLE: bool = false;

            type Base = ::rustpython_vm::builtins::PyTuple;
        }

        // StaticType for Python type
        impl ::rustpython_vm::class::StaticType for #pytype_ident {
            fn static_cell() -> &'static ::rustpython_vm::common::static_cell::StaticCell<::rustpython_vm::builtins::PyTypeRef> {
                ::rustpython_vm::common::static_cell! {
                    static CELL: ::rustpython_vm::builtins::PyTypeRef;
                }
                &CELL
            }

            fn static_baseclass() -> &'static ::rustpython_vm::Py<::rustpython_vm::builtins::PyType> {
                use ::rustpython_vm::class::StaticType;
                ::rustpython_vm::builtins::PyTuple::static_type()
            }
        }

        // PyPayload - following PyBool pattern (use base type's payload_type_id)
        impl ::rustpython_vm::PyPayload for #pytype_ident {
            #[inline]
            fn payload_type_id() -> ::std::any::TypeId {
                <::rustpython_vm::builtins::PyTuple as ::rustpython_vm::PyPayload>::payload_type_id()
            }

            #[inline]
            fn validate_downcastable_from(obj: &::rustpython_vm::PyObject) -> bool {
                obj.class().fast_issubclass(<Self as ::rustpython_vm::class::StaticType>::static_type())
            }

            fn class(_ctx: &::rustpython_vm::vm::Context) -> &'static ::rustpython_vm::Py<::rustpython_vm::builtins::PyType> {
                <Self as ::rustpython_vm::class::StaticType>::static_type()
            }
        }

        // MaybeTraverse - delegate to inner PyTuple
        impl ::rustpython_vm::object::MaybeTraverse for #pytype_ident {
            fn try_traverse(&self, traverse_fn: &mut ::rustpython_vm::object::TraverseFn<'_>) {
                self.0.try_traverse(traverse_fn)
            }
        }

        // Deref to access inner PyTuple
        impl ::std::ops::Deref for #pytype_ident {
            type Target = ::rustpython_vm::builtins::PyTuple;

            #[inline]
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        // PySubclass for proper inheritance
        impl ::rustpython_vm::class::PySubclass for #pytype_ident {
            type Base = ::rustpython_vm::builtins::PyTuple;

            #[inline]
            fn as_base(&self) -> &Self::Base {
                &self.0
            }
        }

        impl ::rustpython_vm::class::PySubclassTransparent for #pytype_ident {}

        // PyStructSequence trait for Python type
        impl ::rustpython_vm::types::PyStructSequence for #pytype_ident {
            type Data = #data_ident;
        }

        // ToPyObject for Data struct - uses PyStructSequence::from_data
        impl ::rustpython_vm::convert::ToPyObject for #data_ident {
            fn to_pyobject(self, vm: &::rustpython_vm::VirtualMachine) -> ::rustpython_vm::PyObjectRef {
                <#pytype_ident as ::rustpython_vm::types::PyStructSequence>::from_data(self, vm).into()
            }
        }
    };

    Ok(output)
}
