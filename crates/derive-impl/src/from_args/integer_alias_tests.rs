use super::*;

#[test]
fn integer_alias_literal_defaults_stay_native() {
    let types: [Type; 4] = [
        parse_quote!(PySsize),
        parse_quote!(rustpython_vm::function::PySsize),
        parse_quote!(::rustpython_vm::function::PySsize),
        parse_quote!(crate::vm::function::PySsize),
    ];
    let defaults: [(Expr, i128); 3] = [
        (parse_quote!(0), 0),
        (parse_quote!(-1), -1),
        (parse_quote!((-1)), -1),
    ];
    for ty in types {
        assert!(is_primitive_int(&ty));
        for (default, value) in &defaults {
            let field: Field = parse_quote! {
                #[pyarg(any, default = #default)]
                max_length: #ty
            };
            let tokens = generate_field((0, &field)).unwrap().to_string();
            assert!(tokens.contains(&quote!(unwrap_or_else(|| #default)).to_string()));
            assert!(!tokens.contains("ToPyObject"));
            assert!(
                tokens.contains(
                    &quote!(::rustpython_vm::convert::TryFromObject::try_from_object(
                        vm, x
                    ))
                    .to_string()
                )
            );
            let attr = ArgAttribute::try_from(&field).unwrap();
            let repr = repr_path();
            assert_eq!(
                signature_default(&field, &attr).unwrap().to_string(),
                quote!(Some(#repr::Int(#value))).to_string()
            );
        }
    }
}

#[test]
fn integer_alias_defaults_have_integer_metadata() {
    let repr = repr_path();
    let field: Field = parse_quote! {
        #[pyarg(any, default)]
        max_length: PySsize
    };
    let attr = ArgAttribute::try_from(&field).unwrap();
    assert_eq!(
        signature_default(&field, &attr).unwrap().to_string(),
        quote!(Some(#repr::Int(0))).to_string()
    );

    let field: Field = parse_quote! {
        #[pyarg(any, default = MAX_LENGTH)]
        max_length: PySsize
    };
    let attr = ArgAttribute::try_from(&field).unwrap();
    assert_eq!(
        signature_default(&field, &attr).unwrap().to_string(),
        quote!(Some(#repr::Int((MAX_LENGTH) as i128))).to_string()
    );
}

#[test]
fn integer_alias_recognition_does_not_include_wrappers_or_unknown_aliases() {
    let types: [Type; 5] = [
        parse_quote!(OptionalArg<PySsize>),
        parse_quote!(Option<PySsize>),
        parse_quote!(OtherSize),
        parse_quote!(PySsize<T>),
        parse_quote!(<T as Trait>::PySsize),
    ];
    for ty in types {
        assert!(!is_primitive_int(&ty));
    }
    let field: Field = parse_quote! {
        #[pyarg(any, optional)]
        max_length: OptionalArg<PySsize>
    };
    let tokens = generate_field((0, &field)).unwrap().to_string();
    assert!(
        tokens.contains(&quote!(unwrap_or_else(|| ::std::default::Default::default())).to_string())
    );
    assert!(!tokens.contains("ToPyObject"));
    let attr = ArgAttribute::try_from(&field).unwrap();
    assert_eq!(
        signature_default(&field, &attr).unwrap().to_string(),
        quote!(Some(
            <OptionalArg<PySsize> as ::rustpython_vm::function::OptionalArgDefault>::PY_DEFAULT
        ))
        .to_string()
    );

    let field: Field = parse_quote! {
        #[pyarg(any, default = -1)]
        max_length: OtherSize
    };
    let tokens = generate_field((0, &field)).unwrap().to_string();
    assert!(tokens.contains("ToPyObject"));
    assert!(tokens.contains("< OtherSize as :: rustpython_vm :: convert :: TryFromObject >"));
}
