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

macro_rules! py_items {
    ($ctx:ident, $mac:ident, $thru:tt,) => {};
    ($ctx:ident, $mac:ident, $thru:tt, struct $name:ident {$($inner:tt)*} $($rest:tt)*) => {
        $mac!(
            @py_item
            $ctx,
            ($name, __py_class!($ctx, $name($ctx.object()), { $($inner)* })),
            $thru
        );
        py_items!($ctx, $mac, $thru, $($rest)*)
    };
    (
        $ctx:ident,
        $mac:ident,
        $thru:tt,
        struct $name:ident($parent:ident) { $($inner:tt)* }
        $($rest:tt)*
    ) => {
        $mac!(
            @py_item
            $ctx,
            ($name, __py_class!($ctx, $name($parent.clone()), { $($inner)* })),
            $thru
        );
        py_items!($ctx, $mac, $thru, $($rest)*)
    };
    ($ctx:ident, $mac:ident, $thru:tt, fn $func:ident; $($rest:tt)*) => {
        $mac!(
            @py_item
            $ctx,
            ($func, $ctx.new_rustfunc($func)),
            $thru
        );
        py_items!($ctx, $mac, $thru, $($rest)*)
    };
    ($ctx:ident, $mac:ident, $thru:tt, fn $name:ident = $func:expr; $($rest:tt)*) => {
        $mac!(
            @py_item
            $ctx,
            ($name, $ctx.new_rustfunc($func)),
            $thru
        );
        py_items!($ctx, $mac, $thru, $($rest)*)
    };
    ($ctx:ident, $mac:ident, $thru:tt, mod $name:ident { $($inner:tt)* } $($rest:tt)*) => {
        $mac!(
            @py_item
            $ctx,
            ($name, py_module!($ctx, $name { $($inner)* })),
            $thru
        );
        py_items!($ctx, $mac, $thru, $($rest)*)
    };
    (
        $ctx:ident,
        $mac:ident,
        $thru:tt,
        mod $name:ident($parent:ident) { $($inner:tt)* }
        $($rest:tt)*
    ) => {
        $mac!(
            @py_item
            $ctx,
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
        $mac!(
            @py_item
            $ctx,
            ($name, $value),
            $thru
        );
        py_items!($ctx, $mac, $thru, $($rest)*)
    };
}

#[allow(unused)]
macro_rules! __py_item {
    (@py_item $ctx:ident, ($name:ident, $item:expr), $var:ident) => {
        let $var = $item;
    };
}

#[macro_export]
macro_rules! py_item {
    ($ctx:expr, $($item:tt)*) => {{
        let __ctx: &PyContext = $ctx;
        py_items!(__ctx, __py_item, __var, $($item)*);
        __var
    }};
}

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
    }
}

#[macro_export]
macro_rules! py_class {
    (
        $ctx:expr,
        $name:ident($parent:ident) { $($attr_name:ident: ($($attr_val:tt)*)),*$(,)* }
    ) => {
        __py_class!($ctx, $name($parent.clone()), {$($attr_name: ($($attr_val)*)),*})
    };
    ($ctx:expr, $name:ident { $($attr_name:ident: ($($attr_val:tt)*)),*$(,)* }) => {{
        let __ctx: &PyContext = $ctx;
        __py_class!(__ctx, $name(__ctx.object()), {$($attr_name: ($($attr_val)*)),*})
    }};
}

#[macro_export]
macro_rules! py_get_item {
    ($val:ident.$($attr:ident).*) => {
        py_get_item!(($val).$($attr).*)
    };
    (($val:expr).$attr:ident.$($rest:ident).*) => {
        match $val.get_item(stringify!($attr)) {
            Some(val) => py_get_item!((val).$($rest).*),
            None => None,
        }
    };
    (($val:expr).$attr:ident) => {
        py_get_item!(($val).$attr.)
    };
    (($val:expr).) => {
        Some($val)
    };
}
