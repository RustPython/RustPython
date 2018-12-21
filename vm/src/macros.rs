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

macro_rules! __py_attr {
    ($ctx:expr, $name:ident, func $func:expr) => {
        $ctx.new_rustfunc($func)
    };
    ($ctx:expr, $name:ident, class { $($attr_name:ident: ($($attr_val:tt)*)),*$(,)* }) => {
        py_class!($ctx, $name {$($attr_name: ($($attr_val)*)),*})
    };
    (
        $ctx:expr,
        $name:ident,
        class($parent:ident) { $($attr_name:ident: ($($attr_val:tt)*)),*$(,)* }
    ) => {
        py_class!($ctx, $name($parent) {$($attr_name: ($($attr_val)*)),*})
    };
    ($ctx:expr, $name:ident, $attr:expr) => {
        $attr
    };
}

macro_rules! __py_module {
    (
        $ctx:expr,
        $name:ident($parent:expr),
        { $($attr_name:ident: ($($attr_val:tt)*)),*$(,)* }
    ) => {{
        let __ctx: &PyContext = $ctx;
        let __py_mod = __ctx.new_module(&stringify!(ident).to_string(), __ctx.new_scope($parent));
        $(
            #[allow(non_snake_case)]
            let $attr_name = __py_attr!(__ctx, $attr_name, $($attr_val)*);
            __ctx.set_attr(&__py_mod, stringify!($attr_name), $attr_name.clone());
        )*
        __py_mod
    }};
}

#[macro_export]
macro_rules! py_module {
    (
        $ctx:expr,
        $name:ident($parent:expr) { $($attr_name:ident: ($($attr_val:tt)*)),*$(,)* }
    ) => {
        __py_module!($ctx, $name(Some($parent)), { $($attr_name: ($($attr_val)*)),* })
    };
    ($ctx:expr, $name:ident {$($attr_name:ident: ($($attr_val:tt)*)),*$(,)* }) => {
        __py_module!($ctx, $name(None), {$($attr_name: ($($attr_val)*)),*})
    }

}

macro_rules! __py_class {
    (
        $ctx:expr,
        $name:ident($parent:expr),
        { $($attr_name:ident: ($($attr_val:tt)*)),*$(,)* }
    ) => {{
        let __ctx: &PyContext = $ctx;
        let __py_class = __ctx.new_class(stringify!($name), $parent);
        $(
            #[allow(non_snake_case)]
            let $attr_name = __py_attr!(__ctx, $attr_name, $($attr_val)*);
            __ctx.set_attr(&__py_class, stringify!($attr_name), $attr_name.clone());
        )*
        __py_class
    }};
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
