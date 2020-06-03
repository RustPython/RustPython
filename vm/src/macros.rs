// count number of tokens given as arguments.
// see: https://danielkeep.github.io/tlborm/book/blk-counting.html
#[macro_export]
macro_rules! replace_expr {
    ($_t:tt $sub:expr) => {
        $sub
    };
}

#[macro_export]
macro_rules! count_tts {
    ($($tts:tt)*) => {0usize $(+ $crate::replace_expr!($tts 1usize))*};
}

#[macro_export]
macro_rules! type_check {
    ($vm:ident, $args:ident, $arg_count:ident, $arg_name:ident, $arg_type:expr) => {
        // None indicates that we have no type requirement (i.e. we accept any type)
        if let Some(expected_type) = $arg_type {
            let arg = &$args.args[$arg_count];

            if !$crate::obj::objtype::isinstance(arg, &expected_type) {
                use $crate::pyobject::TypeProtocol;

                let arg_typ = arg.class();
                let expected_type_name = $vm.to_pystr(&expected_type)?;
                let actual_type = $vm.to_pystr(&arg_typ)?;
                return Err($vm.new_type_error(format!(
                    "argument of type {} is required for parameter {} ({}) (got: {})",
                    expected_type_name,
                    $arg_count + 1,
                    stringify!($arg_name),
                    actual_type
                )));
            }
        }
    };
}

#[macro_export]
macro_rules! arg_check {
    ( $vm: ident, $args:ident ) => {
        // Zero-arg case
        if $args.args.len() != 0 {
            return Err($vm.new_type_error(format!(
                "Expected no arguments (got: {})", $args.args.len())));
        }
    };
    ( $vm: ident, $args:ident, required=[$( ($arg_name:ident, $arg_type:expr) ),*] ) => {
        $crate::arg_check!($vm, $args, required=[$( ($arg_name, $arg_type) ),*], optional=[]);
    };
    ( $vm: ident, $args:ident, required=[$( ($arg_name:ident, $arg_type:expr) ),*], optional=[$( ($optional_arg_name:ident, $optional_arg_type:expr) ),*] ) => {
        let mut arg_count = 0;

        // use macro magic to compile-time count number of required and optional arguments
        let minimum_arg_count = $crate::count_tts!($($arg_name)*);
        let maximum_arg_count = minimum_arg_count + $crate::count_tts!($($optional_arg_name)*);

        // verify that the number of given arguments is right
        if $args.args.len() < minimum_arg_count || $args.args.len() > maximum_arg_count {
            let expected_str = if minimum_arg_count == maximum_arg_count {
                format!("{}", minimum_arg_count)
            } else {
                format!("{}-{}", minimum_arg_count, maximum_arg_count)
            };
            return Err($vm.new_type_error(format!(
                "Expected {} arguments (got: {})",
                expected_str,
                $args.args.len()
            )));
        };

        // for each required parameter:
        //  check if the type matches. If not, return with error
        //  assign the arg to a variable
        $(
            $crate::type_check!($vm, $args, arg_count, $arg_name, $arg_type);
            let $arg_name = &$args.args[arg_count];
            #[allow(unused_assignments)]
            {
                arg_count += 1;
            }
        )*

        // for each optional parameter, if there are enough positional arguments:
        //  check if the type matches. If not, return with error
        //  assign the arg to a variable
        $(
            let $optional_arg_name = if arg_count < $args.args.len() {
                $crate::type_check!($vm, $args, arg_count, $optional_arg_name, $optional_arg_type);
                let ret = Some(&$args.args[arg_count]);
                #[allow(unused_assignments)]
                {
                    arg_count += 1;
                }
                ret
            } else {
                None
            };
        )*
    };
}

#[macro_export]
macro_rules! no_kwargs {
    ( $vm: ident, $args:ident ) => {
        // Zero-arg case
        if $args.kwargs.len() != 0 {
            return Err($vm.new_type_error(format!(
                "Expected no keyword arguments (got: {})",
                $args.kwargs.len()
            )));
        }
    };
}

#[macro_export]
macro_rules! py_module {
    ( $vm:expr, $module_name:expr, { $($name:expr => $value:expr),* $(,)? }) => {{
        let module = $vm.new_module($module_name, $vm.ctx.new_dict());
        $crate::extend_module!($vm, module, { $($name => $value),* });
        module
    }};
}

#[macro_export]
macro_rules! extend_module {
    ( $vm:expr, $module:expr, { $($name:expr => $value:expr),* $(,)? }) => {{
        #[allow(unused_variables)]
        let module: &$crate::pyobject::PyObjectRef = &$module;
        $(
            $vm.__module_set_attr(&module, $name, $value).unwrap();
        )*
    }};
}

#[macro_export]
macro_rules! py_class {
    ( $ctx:expr, $class_name:expr, $class_base:expr, { $($name:tt => $value:expr),* $(,)* }) => {
        {
            let py_class = $ctx.new_class($class_name, $class_base);
            // FIXME: setting flag here probably wrong
            py_class.slots.write().unwrap().flags |= $crate::slots::PyTpFlags::BASETYPE;
            $crate::extend_class!($ctx, &py_class, { $($name => $value),* });
            py_class
        }
    }
}

#[macro_export]
macro_rules! extend_class {
    ( $ctx:expr, $class:expr, { $($name:tt => $value:expr),* $(,)* }) => {
        $(
            $crate::extend_class!(@set_attr($ctx, $class, $name, $value));
        )*
        $ctx.add_tp_new_wrapper(&$class);
    };

    (@set_attr($ctx:expr, $class:expr, (slot $slot_name:ident), $value:expr)) => {
        $class.slots.write().unwrap().$slot_name = Some(
            $crate::function::IntoPyNativeFunc::into_func($value)
        );
    };
    (@set_attr($ctx:expr, $class:expr, $name:expr, $value:expr)) => {
        $class.set_str_attr($name, $value);
    };
}

#[macro_export]
macro_rules! py_namespace {
    ( $vm:expr, { $($name:expr => $value:expr),* $(,)* }) => {
        {
            let namespace = $vm.ctx.new_namespace();
            $(
                $vm.__module_set_attr(&namespace, $name, $value).unwrap();
            )*
            namespace
        }
    }
}

/// Macro to match on the built-in class of a Python object.
///
/// Like `match`, `match_class!` must be exhaustive, so a default arm with
/// the uncasted object is required.
///
/// # Examples
///
/// ```
/// use num_bigint::ToBigInt;
/// use num_traits::Zero;
///
/// use rustpython_vm::VirtualMachine;
/// use rustpython_vm::match_class;
/// use rustpython_vm::obj::objfloat::PyFloat;
/// use rustpython_vm::obj::objint::PyInt;
/// use rustpython_vm::pyobject::PyValue;
///
/// let vm: VirtualMachine = Default::default();
/// let obj = PyInt::new(0).into_ref(&vm).into_object();
/// assert_eq!(
///     "int",
///     match_class!(match obj.clone() {
///         PyInt => "int",
///         PyFloat => "float",
///         _ => "neither",
///     })
/// );
///
/// ```
///
/// With a binding to the downcasted type:
///
/// ```
/// use num_bigint::ToBigInt;
/// use num_traits::Zero;
///
/// use rustpython_vm::VirtualMachine;
/// use rustpython_vm::match_class;
/// use rustpython_vm::obj::objfloat::PyFloat;
/// use rustpython_vm::obj::objint::PyInt;
/// use rustpython_vm::pyobject::PyValue;
///
/// let vm: VirtualMachine = Default::default();
/// let obj = PyInt::new(0).into_ref(&vm).into_object();
///
/// let int_value = match_class!(match obj {
///     i @ PyInt => i.as_bigint().clone(),
///     f @ PyFloat => f.to_f64().to_bigint().unwrap(),
///     obj => panic!("non-numeric object {}", obj),
/// });
///
/// assert!(int_value.is_zero());
/// ```
#[macro_export]
macro_rules! match_class {
    // The default arm.
    (match ($obj:expr) { _ => $default:expr $(,)? }) => {
        $default
    };

    // The default arm, binding the original object to the specified identifier.
    (match ($obj:expr) { $binding:ident => $default:expr $(,)? }) => {{
        let $binding = $obj;
        $default
    }};
    (match ($obj:expr) { ref $binding:ident => $default:expr $(,)? }) => {{
        let $binding = &$obj;
        $default
    }};

    // An arm taken when the object is an instance of the specified built-in
    // class and binding the downcasted object to the specified identifier and
    // the target expression is a block.
    (match ($obj:expr) { $binding:ident @ $class:ty => $expr:block $($rest:tt)* }) => {
        $crate::match_class!(match ($obj) { $binding @ $class => ($expr), $($rest)* })
    };
    (match ($obj:expr) { ref $binding:ident @ $class:ty => $expr:block $($rest:tt)* }) => {
        $crate::match_class!(match ($obj) { ref $binding @ $class => ($expr), $($rest)* })
    };

    // An arm taken when the object is an instance of the specified built-in
    // class and binding the downcasted object to the specified identifier.
    (match ($obj:expr) { $binding:ident @ $class:ty => $expr:expr, $($rest:tt)* }) => {
        match $obj.downcast::<$class>() {
            Ok($binding) => $expr,
            Err(_obj) => $crate::match_class!(match (_obj) { $($rest)* }),
        }
    };
    (match ($obj:expr) { ref $binding:ident @ $class:ty => $expr:expr, $($rest:tt)* }) => {
        match $obj.payload::<$class>() {
            Some($binding) => $expr,
            None => $crate::match_class!(match ($obj) { $($rest)* }),
        }
    };

    // An arm taken when the object is an instance of the specified built-in
    // class and the target expression is a block.
    (match ($obj:expr) { $class:ty => $expr:block $($rest:tt)* }) => {
        $crate::match_class!(match ($obj) { $class => ($expr), $($rest)* })
    };

    // An arm taken when the object is an instance of the specified built-in
    // class.
    (match ($obj:expr) { $class:ty => $expr:expr, $($rest:tt)* }) => {
        if $obj.payload_is::<$class>() {
            $expr
        } else {
            $crate::match_class!(match ($obj) { $($rest)* })
        }
    };

    // To allow match expressions without parens around the match target
    (match $($rest:tt)*) => {
        $crate::match_class!(@parse_match () ($($rest)*))
    };
    (@parse_match ($($target:tt)*) ({ $($inner:tt)* })) => {
        $crate::match_class!(match ($($target)*) { $($inner)* })
    };
    (@parse_match ($($target:tt)*) ($next:tt $($rest:tt)*)) => {
        $crate::match_class!(@parse_match ($($target)* $next) ($($rest)*))
    };
}

/// Super detailed logging. Might soon overflow your logbuffers
/// Default, this logging is discarded, except when a the `vm-tracing-logging`
/// build feature is enabled.
macro_rules! vm_trace {
    ($($arg:tt)+) => {
        #[cfg(feature = "vm-tracing-logging")]
        trace!($($arg)+);
    }
}

macro_rules! flame_guard {
    ($name:expr) => {
        #[cfg(feature = "flame-it")]
        let _guard = ::flame::start_guard($name);
    };
}

#[macro_export]
macro_rules! class_or_notimplemented {
    ($vm:expr, $t:ty, $obj:expr) => {
        match $crate::pyobject::PyObject::downcast::<$t>($obj) {
            Ok(pyref) => pyref,
            Err(_) => return Ok($vm.ctx.not_implemented()),
        }
    };
}

#[macro_export]
macro_rules! named_function {
    ($ctx:expr, $module:ident, $func:ident) => {{
        paste::expr! {
            $crate::pyobject::PyContext::new_function_named(
                &$ctx,
                [<$module _ $func>],
                stringify!($module).to_owned(),
                stringify!($func).to_owned(),
            )
        }
    }};
}
