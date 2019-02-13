// see: https://danielkeep.github.io/tlborm/book/blk-counting.html
macro_rules! replace_expr {
    ($_t:tt $sub:expr) => {
        $sub
    };
}

macro_rules! count_tts {
    ($($tts:tt)*) => {0usize $(+ replace_expr!($tts 1usize))*};
}

macro_rules! type_check {
    ($vm:ident, $args:ident, $arg_count:ident, $arg_name:ident, $arg_type:expr) => {
        // None indicates that we have no type requirement (i.e. we accept any type)
        if let Some(expected_type) = $arg_type {
            let arg = &$args.args[$arg_count];
            if !objtype::isinstance(arg, &expected_type) {
                let arg_typ = arg.typ();
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
        let mut arg_count = 0;

        let minimum_arg_count = count_tts!($($arg_name)*);
        let maximum_arg_count = minimum_arg_count + count_tts!($($optional_arg_name)*);

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

        $(
            type_check!($vm, $args, arg_count, $arg_name, $arg_type);
            let $arg_name = &$args.args[arg_count];
            #[allow(unused_assignments)]
            {
                arg_count += 1;
            }
        )*

        $(
            let $optional_arg_name = if arg_count < $args.args.len() {
                type_check!($vm, $args, arg_count, $optional_arg_name, $optional_arg_type);
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
