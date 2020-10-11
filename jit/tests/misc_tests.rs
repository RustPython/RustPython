use rustpython_jit::{AbiValue, JitArgumentError};

// TODO currently broken
// #[test]
// fn test_no_return_value() {
//     let func = jit_function! { func() => r##"
//         def func():
//             pass
//     "## };
//
//     assert_eq!(func(), Ok(()));
// }

#[test]
fn test_invoke() {
    let func = jit_function! { func => r##"
        def func(a: int, b: float):
            return 1
    "## };

    assert_eq!(
        func.invoke(&[AbiValue::Int(1)]),
        Err(JitArgumentError::WrongNumberOfArguments)
    );
    assert_eq!(
        func.invoke(&[AbiValue::Int(1), AbiValue::Float(2.0), AbiValue::Int(0)]),
        Err(JitArgumentError::WrongNumberOfArguments)
    );
    assert_eq!(
        func.invoke(&[AbiValue::Int(1), AbiValue::Int(1)]),
        Err(JitArgumentError::ArgumentTypeMismatch)
    );
    assert_eq!(
        func.invoke(&[AbiValue::Int(1), AbiValue::Float(2.0)]),
        Ok(Some(AbiValue::Int(1)))
    );
}

#[test]
fn test_args_builder() {
    let func = jit_function! { func=> r##"
        def func(a: int, b: float):
            return 1
    "## };

    let mut args_builder = func.args_builder();
    assert_eq!(args_builder.set(0, AbiValue::Int(1)), Ok(()));
    assert!(args_builder.is_set(0));
    assert!(!args_builder.is_set(1));
    assert_eq!(
        args_builder.set(1, AbiValue::Int(1)),
        Err(JitArgumentError::ArgumentTypeMismatch)
    );
    assert!(args_builder.is_set(0));
    assert!(!args_builder.is_set(1));
    assert!(args_builder.into_args().is_none());

    let mut args_builder = func.args_builder();
    assert_eq!(args_builder.set(0, AbiValue::Int(1)), Ok(()));
    assert_eq!(args_builder.set(1, AbiValue::Float(1.0)), Ok(()));
    assert!(args_builder.is_set(0));
    assert!(args_builder.is_set(1));

    let args = args_builder.into_args();
    assert!(args.is_some());
    assert_eq!(args.unwrap().invoke(), Some(AbiValue::Int(1)));
}

#[test]
fn test_if_else() {
    let if_else = jit_function! { if_else(a:i64) -> i64 => r##"
        def if_else(a: int):
            if a:
                return 42
            else:
                return 0

            # Prevent type failure from implicit `return None`
            return 0
    "## };

    assert_eq!(if_else(0), Ok(0));
    assert_eq!(if_else(1), Ok(42));
    assert_eq!(if_else(-1), Ok(42));
    assert_eq!(if_else(100), Ok(42));
}

#[test]
fn test_while_loop() {
    let while_loop = jit_function! { while_loop(a:i64) -> i64 => r##"
        def while_loop(a: int):
            b = 0
            while a > 0:
                b += 1
                a -= 1
            return b
    "## };

    assert_eq!(while_loop(0), Ok(0));
    assert_eq!(while_loop(-1), Ok(0));
    assert_eq!(while_loop(1), Ok(1));
    assert_eq!(while_loop(10), Ok(10));
}
