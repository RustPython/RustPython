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
        py_class!($ctx, $class_name, $class_base, $crate::slots::PyTpFlags::BASETYPE, { $($name => $value),* })
    };
    ( $ctx:expr, $class_name:expr, $class_base:expr, $flags:expr, { $($name:tt => $value:expr),* $(,)* }) => {
        {
            #[allow(unused_mut)]
            let mut slots = $crate::slots::PyTypeSlots::from_flags($crate::slots::PyTpFlags::DEFAULT | $flags);
            $($crate::py_class!(@extract_slots($ctx, &mut slots, $name, $value));)*
            let py_class = $ctx.new_class($class_name, $class_base, slots);
            $($crate::py_class!(@extract_attrs($ctx, &py_class, $name, $value));)*
            $ctx.add_slot_wrappers(&py_class);
            py_class
        }
    };
    (@extract_slots($ctx:expr, $slots:expr, (slot new), $value:expr)) => {
        $slots.new = Some(
            $crate::function::IntoPyNativeFunc::into_func($value)
        );
    };
    (@extract_slots($ctx:expr, $slots:expr, (slot $slot_name:ident), $value:expr)) => {
        $slots.$slot_name.store(Some($value));
    };
    (@extract_slots($ctx:expr, $class:expr, $name:expr, $value:expr)) => {};
    (@extract_attrs($ctx:expr, $slots:expr, (slot $slot_name:ident), $value:expr)) => {};
    (@extract_attrs($ctx:expr, $class:expr, $name:expr, $value:expr)) => {
        $class.set_str_attr($name, $value);
    };
}

#[macro_export]
macro_rules! extend_class {
    ( $ctx:expr, $class:expr, { $($name:expr => $value:expr),* $(,)* }) => {
        $(
            $class.set_str_attr($name, $value);
        )*
        $ctx.add_slot_wrappers(&$class);
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
/// use rustpython_vm::match_class;
/// use rustpython_vm::builtins::PyFloat;
/// use rustpython_vm::builtins::PyInt;
/// use rustpython_vm::pyobject::PyValue;
///
/// # rustpython_vm::Interpreter::default().enter(|vm| {
/// let obj = PyInt::from(0).into_ref(&vm).into_object();
/// assert_eq!(
///     "int",
///     match_class!(match obj.clone() {
///         PyInt => "int",
///         PyFloat => "float",
///         _ => "neither",
///     })
/// );
/// # });
///
/// ```
///
/// With a binding to the downcasted type:
///
/// ```
/// use num_bigint::ToBigInt;
/// use num_traits::Zero;
///
/// use rustpython_vm::match_class;
/// use rustpython_vm::builtins::PyFloat;
/// use rustpython_vm::builtins::PyInt;
/// use rustpython_vm::pyobject::{PyValue, BorrowValue};
///
/// # rustpython_vm::Interpreter::default().enter(|vm| {
/// let obj = PyInt::from(0).into_ref(&vm).into_object();
///
/// let int_value = match_class!(match obj {
///     i @ PyInt => i.borrow_value().clone(),
///     f @ PyFloat => f.to_f64().to_bigint().unwrap(),
///     obj => panic!("non-numeric object {}", obj),
/// });
///
/// assert!(int_value.is_zero());
/// # });
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
    ($t:ty, $obj:expr) => {
        match $crate::pyobject::PyObjectRef::downcast_ref::<$t>($obj) {
            Some(pyref) => pyref,
            None => return Ok($crate::pyobject::PyArithmaticValue::NotImplemented),
        }
    };
}

#[macro_export]
macro_rules! named_function {
    ($ctx:expr, $module:ident, $func:ident) => {{
        #[allow(unused_variables)] // weird lint, something to do with paste probably
        let ctx: &$crate::pyobject::PyContext = &$ctx;
        $crate::__exports::paste::expr! {
            ctx.make_funcdef(
                stringify!($module),
                [<$module _ $func>],
            )
            .into_function()
            .with_module(ctx.new_str(stringify!($func).to_owned()))
            .build(ctx)
        }
    }};
}

// can't use PyThreadingConstraint for stuff like this since it's not an auto trait, and
// therefore we can't add it ad-hoc to a trait object
cfg_if::cfg_if! {
    if #[cfg(feature = "threading")] {
        macro_rules! py_dyn_fn {
            (dyn Fn($($arg:ty),*$(,)*) -> $ret:ty) => {
                dyn Fn($($arg),*) -> $ret + Send + Sync + 'static
            };
        }
    } else {
        macro_rules! py_dyn_fn {
            (dyn Fn($($arg:ty),*$(,)*) -> $ret:ty) => {
                dyn Fn($($arg),*) -> $ret + 'static
            };
        }
    }
}

/// A modified version of the hashmap! macro from the maplit crate
macro_rules! hashmap {
    (@single $($x:tt)*) => (());
    (@count $($rest:expr),*) => (<[()]>::len(&[$(hashmap!(@single $rest)),*]));

    (hasher=$hasher:expr, $($key:expr => $value:expr,)+) => { hashmap!(hasher=$hasher, $($key => $value),+) };
    (hasher=$hasher:expr, $($key:expr => $value:expr),*) => {
        {
            let _cap = hashmap!(@count $($key),*);
            let mut _map = ::std::collections::HashMap::with_capacity_and_hasher(_cap, $hasher);
            $(
                let _ = _map.insert($key, $value);
            )*
            _map
        }
    };
}
