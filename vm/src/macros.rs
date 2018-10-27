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
                        let arg_typ = arg.typ().clone();
                        let actual_type = arg_typ.borrow().str().clone();
                        return Err($vm.new_type_error(format!(
                            "argument of type {} is required for parameter {} ({}) (got: {})",
                            expected_type.borrow().str(),
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
