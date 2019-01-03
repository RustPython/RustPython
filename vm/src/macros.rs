macro_rules! arg_check {
    ( $vm: ident, $args:ident ) => {
        // Zero-arg case
        if $args.args.len() != 0 {
            return Err($vm.new_type_error(format!(
                "Expected no arguments (got: {})", $args.args.len())));
        }
    };
    ( $vm: ident, $args:ident, required=[$( ($arg_name:ident, $arg_type:expr) ),*] ) => {
        arg_check!($vm, $args, required=[$( ($arg_name, $arg_type) ),*], optional=[]);
    };
    ( $vm: ident, $args:ident, required=[$( ($arg_name:ident, $arg_type:expr) ),*], optional=[$( ($optional_arg_name:ident, $optional_arg_type:expr) ),*] ) => {
        let mut expected_args: Vec<(usize, &str, Option<PyObjectRef>)> = vec![];
        let mut arg_count = 0;

        $(
            if arg_count >= $args.args.len() {
                // TODO: Report the number of expected arguments
                return Err($vm.new_type_error(format!(
                    "Expected more arguments (got: {})",
                    $args.args.len()
                )));
            }
            expected_args.push((arg_count, stringify!($arg_name), $arg_type));
            let $arg_name = &$args.args[arg_count];
            #[allow(unused_assignments)]
            {
                arg_count += 1;
            }
        )*

        let minimum_arg_count = arg_count;

        $(
            let $optional_arg_name = if arg_count < $args.args.len() {
                expected_args.push((arg_count, stringify!($optional_arg_name), $optional_arg_type));
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

        if $args.args.len() < minimum_arg_count || $args.args.len() > expected_args.len() {
            let expected_str = if minimum_arg_count == arg_count {
                format!("{}", arg_count)
            } else {
                format!("{}-{}", minimum_arg_count, arg_count)
            };
            return Err($vm.new_type_error(format!(
                "Expected {} arguments (got: {})",
                expected_str,
                $args.args.len()
            )));
        };

        for (arg, (arg_position, arg_name, expected_type)) in
            $args.args.iter().zip(expected_args.iter())
        {
            match expected_type {
                Some(expected_type) => {
                    if !objtype::isinstance(arg, &expected_type) {
                        let arg_typ = arg.typ();
                        let expected_type_name = $vm.to_pystr(expected_type)?;
                        let actual_type = $vm.to_pystr(&arg_typ)?;
                        return Err($vm.new_type_error(format!(
                            "argument of type {} is required for parameter {} ({}) (got: {})",
                            expected_type_name,
                            arg_position + 1,
                            arg_name,
                            actual_type
                        )));
                    }
                }
                // None indicates that we have no type requirement (i.e. we accept any type)
                None => {}
            }
        }
    };
}

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

// TODO: Allow passing a module name, so you could have a module's name be, e.g. `_ast.FunctionDef`

#[macro_export]
#[doc(hidden)]
macro_rules! py_items {
    ($ctx:ident, $mac:ident, $thru:tt,) => {};
    ($ctx:ident, $mac:ident, $thru:tt, struct $name:ident {$($inner:tt)*} $($rest:tt)*) => {
        __item_mac!(
            $ctx,
            $mac,
            ($name, __py_class!($ctx, $name($ctx.object()), { $($inner)* })),
            $thru
        );
        py_items!($ctx, $mac, $thru, $($rest)*)
    };
    (
        $ctx:ident,
        $mac:ident,
        $thru:tt,
        struct $name:ident($parent:expr) { $($inner:tt)* }
        $($rest:tt)*
    ) => {
        __item_mac!(
            $ctx,
            $mac,
            ($name, __py_class!($ctx, $name($parent.clone()), { $($inner)* })),
            $thru
        );
        py_items!($ctx, $mac, $thru, $($rest)*)
    };
    ($ctx:ident, $mac:ident, $thru:tt, fn $func:ident; $($rest:tt)*) => {
        __item_mac!(
            $ctx,
            $mac,
            ($func, $ctx.new_rustfunc($func)),
            $thru
        );
        py_items!($ctx, $mac, $thru, $($rest)*)
    };
    ($ctx:ident, $mac:ident, $thru:tt, fn $name:ident = $func:expr; $($rest:tt)*) => {
        __item_mac!(
            $ctx,
            $mac,
            ($name, $ctx.new_rustfunc($func)),
            $thru
        );
        py_items!($ctx, $mac, $thru, $($rest)*)
    };
    ($ctx:ident, $mac:ident, $thru:tt, mod $name:ident { $($inner:tt)* } $($rest:tt)*) => {
        __item_mac!(
            $ctx,
            $mac,
            ($name, py_module!($ctx, $name { $($inner)* })),
            $thru
        );
        py_items!($ctx, $mac, $thru, $($rest)*)
    };
    (
        $ctx:ident,
        $mac:ident,
        $thru:tt,
        mod $name:ident($parent:expr) { $($inner:tt)* }
        $($rest:tt)*
    ) => {
        __item_mac!(
            $ctx,
            $mac,
            ($name, py_module!($ctx, $name($parent) { $($inner)* })),
            $thru
        );
        py_items!($ctx, $mac, $thru, $($rest)*)
    };
    (
        $ctx:ident,
        $mac:ident,
        $thru:tt,
        let $name:ident = $value:expr;
        $($rest:tt)*
    ) => {
        __item_mac!(
            $ctx,
            $mac,
            ($name, $value),
            $thru
        );
        py_items!($ctx, $mac, $thru, $($rest)*)
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! __item_mac {
    // To catch keywords like type or ref
    ($ctx:ident, $mac:ident, (as, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (as, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (use, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (use, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (extern, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (extern, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (break, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (break, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (const, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (const, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (continue, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (continue, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (crate, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (crate, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (dyn, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (dyn, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (else, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (else, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (if, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (if, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (enum, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (enum, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (extern, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (extern, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (false, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (false, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (fn, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (fn, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (for, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (for, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (if, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (if, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (impl, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (impl, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (in, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (in, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (for, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (for, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (let, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (let, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (loop, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (loop, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (match, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (match, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (mod, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (mod, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (move, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (move, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (mut, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (mut, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (pub, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (pub, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (impl, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (impl, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (ref, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (ref, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (return, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (return, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (Self, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (Self, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (self, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (self, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (static, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (static, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (struct, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (struct, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (super, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (super, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (trait, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (trait, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (true, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (true, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (type, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (type, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (unsafe, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (unsafe, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (use, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (use, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (where, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (where, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (while, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (while, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (abstract, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (abstract, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (async, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (async, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (become, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (become, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (box, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (box, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (do, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (do, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (final, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (final, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (macro, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (macro, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (override, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (override, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (priv, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (priv, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (try, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (try, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (typeof, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (typeof, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (unsized, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (unsized, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (virtual, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (virtual, $value), $thru)
    };
    ($ctx:ident, $mac:ident, (yield, $value:expr), $thru:tt) => {
        $mac!(@py_item_kw $ctx, (yield, $value), $thru)
    };
    ($ctx:ident, $mac:ident, ($name:ident, $value:expr), $thru:tt) => {
        $mac!(@py_item $ctx, ($name, $value), $thru)
    };
}

#[allow(unused)]
#[macro_export]
#[doc(hidden)]
macro_rules! __py_item {
    (@py_item $ctx:ident, ($name:ident, $item:expr), $var:ident) => {
        let $var = $item;
    };
    (@py_item_kw $ctx:ident, ($name:ident, $item:expr), $var:ident) => {
        let $var = $item;
    };
}

/// Constructs a Python type.
///
/// ```rust,ignore
/// py_item!(ctx, $py_item)
/// ```
///
/// Possible `$py_item` values:
/// ```rust,ignore
/// mod $mod_name { $items }
/// struct $class_name { $items }
/// struct $class_name($parent_class) { $items }
/// // $rustfunc is of type `Fn(&mut VirtualMachine, PyFuncArgs) -> PyResult`
/// fn $func_name = $rustfunc;
/// // $item_value is a PyObjectRef
/// let $item_name = $item_value;
/// ```
///
/// # Examples
///
/// ```
/// # #[macro_use] extern crate rustpython_vm;
/// # use rustpython_vm::{VirtualMachine, pyobject::{PyContext, PyResult}};
/// # fn test_new(vm: &mut VirtualMachine, args: rustpython_vm::pyobject::PyFuncArgs) -> PyResult {
///     Ok(vm.get_none())
/// # }
/// # fn main() {
/// # let mut vm = VirtualMachine::new();
/// let ctx: PyContext = vm.ctx;
/// let py_mod = py_item!(&ctx, mod test {
///     struct TestClass {
///         fn __new__ = test_new;
///     }
/// });
/// # }
/// ```
#[macro_export]
macro_rules! py_item {
    ($ctx:expr, $($item:tt)*) => {{
        let __ctx: &PyContext = $ctx;
        py_items!(__ctx, __py_item, __var, $($item)*);
        __var
    }};
}

#[macro_export]
#[doc(hidden)]
macro_rules! __py_module {
    (
        $ctx:ident,
        $name:ident($parent:expr),
        { $($item:tt)* }
    ) => {{
        let __py_mod = $ctx.new_module(&stringify!(ident).to_string(), $ctx.new_scope($parent));
        py_items!($ctx, __py_module, __py_mod, $($item)*);
        __py_mod
    }};
    (@py_item $ctx:ident, ($name:ident, $value:expr), $py_mod:ident) => {
        #[allow(non_snake_case)]
        let $name = $value;
        $ctx.set_attr(&$py_mod, stringify!($name), $name.clone());
    };
    (@py_item_kw $ctx:ident, ($name:ident, $value:expr), $py_mod:ident) => {
        $ctx.set_attr(&$py_mod, stringify!($name), $value);
    }
}

#[macro_export]
macro_rules! py_module {
    (
        $ctx:expr,
        $name:ident($parent:expr) { $($item:tt)* }
    ) => {{
        let __ctx: &PyContext = $ctx;
        __py_module!(__ctx, $name(Some($parent)), { $($item)* })
    }};
    ($ctx:expr, $name:ident { $($item:tt)* }) => {{
        let __ctx: &PyContext = $ctx;
        __py_module!(__ctx, $name(None), { $($item)* })
    }}

}

#[macro_export]
#[doc(hidden)]
macro_rules! __py_class {
    (
        $ctx:ident,
        $name:ident($parent:expr),
        { $($item:tt)* }
    ) => {{
        let __py_class = $ctx.new_class(stringify!($name), $parent);
        py_items!($ctx, __py_class, __py_class, $($item)*);
        __py_class
    }};
    (@py_item $ctx:ident, ($name:ident, $value:expr), $py_class:ident) => {
        #[allow(non_snake_case)]
        let $name = $value;
        $ctx.set_attr(&$py_class, stringify!($name), $name.clone());
    };
    (@py_item_kw $ctx:ident, ($name:ident, $value:expr), $py_class:ident) => {
        $ctx.set_attr(&$py_class, stringify!($name), $value);
    }
}

#[macro_export]
macro_rules! py_class {
    (
        $ctx:expr,
        $name:ident($parent:expr) { $($attr_name:ident: ($($attr_val:tt)*)),*$(,)* }
    ) => {
        __py_class!($ctx, $name($parent.clone()), {$($attr_name: ($($attr_val)*)),*})
    };
    ($ctx:expr, $name:ident { $($attr_name:ident: ($($attr_val:tt)*)),*$(,)* }) => {{
        let __ctx: &PyContext = $ctx;
        __py_class!(__ctx, $name(__ctx.object()), {$($attr_name: ($($attr_val)*)),*})
    }};
}

/// Attempts to get the chain of attributes, returning an option that contains the value of
/// the last one.
///
/// ```rust,ignore
/// py_get_item!(($val).$attr1.$attr2.$...)
/// ```
///
/// # Examples
///
/// ```
/// # #[macro_use] extern crate rustpython_vm;
/// # fn main() {
/// # let mut vm = rustpython_vm::VirtualMachine::new();
/// let abs_func = py_get_item!((vm.sys_module).modules.__builtins__.abs);
/// # }
/// ```
#[macro_export]
macro_rules! py_get_item {
    ($val:ident.$($attr:ident).*) => {
        py_get_item!(($val).$($attr).*)
    };
    (($val:expr).$attr:ident.$($rest:ident).*) => {{
        use $crate::pyobject::DictProtocol;
        match $val.get_item(stringify!($attr)) {
            Some(val) => {
                py_get_item!((val).$($rest).*)
            },
            None => None,
        }
    }};
    (($val:expr).$attr:ident) => {
        py_get_item!(($val).$attr.)
    };
    (($val:expr).) => {
        Some($val)
    };
}
