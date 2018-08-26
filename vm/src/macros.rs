macro_rules! arg_check {
    ( $vm: ident, $args:ident ) => {
        // Zero-arg case
        if $args.args.len() != 0 {
            return Err($vm.new_type_error(format!(
                "Expected no arguments (got: {})", $args.args.len())));
        }
    };
    ( $vm: ident, $args:ident, $( ($arg_name:ident, $arg_type:expr) ),* ) => {
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

        if $args.args.len() != expected_args.len() {
            return Err($vm.new_type_error(format!(
                "Expected {} arguments (got: {})",
                expected_args.len(),
                $args.args.len()
            )));
        };

        for (arg, (arg_position, arg_name, expected_type)) in
            $args.args.iter().zip(expected_args.iter())
        {
            match expected_type {
                Some(expected_type) => {
                    if !objtype::isinstance(arg.clone(), expected_type.clone()) {
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
