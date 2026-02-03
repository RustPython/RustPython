use super::{PY_CF_OPTIMIZED_AST, PY_CF_TYPE_COMMENTS, PY_COMPILE_FLAG_AST_ONLY};

#[pymodule]
pub(crate) mod _ast {
    use crate::{
        AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyStrRef, PyTupleRef, PyType, PyTypeRef},
        function::FuncArgs,
        types::{Constructor, Initializer},
    };
    #[pyattr]
    #[pyclass(module = "_ast", name = "AST")]
    #[derive(Debug, PyPayload)]
    pub(crate) struct NodeAst;

    #[pyclass(with(Constructor, Initializer), flags(BASETYPE, HAS_DICT))]
    impl NodeAst {
        #[pyattr]
        fn _fields(ctx: &Context) -> PyTupleRef {
            ctx.empty_tuple.clone()
        }
    }

    impl Constructor for NodeAst {
        type Args = FuncArgs;

        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            // AST nodes accept extra arguments (unlike object.__new__)
            // This matches CPython's behavior where AST has its own tp_new
            let dict = if cls
                .slots
                .flags
                .contains(crate::types::PyTypeFlags::HAS_DICT)
            {
                Some(vm.ctx.new_dict())
            } else {
                None
            };
            let zelf = vm.ctx.new_base_object(cls, dict);

            // Initialize the instance with the provided arguments
            // FIXME: This is probably incorrect. Please check if init should be called outside of __new__
            Self::slot_init(zelf.clone(), args, vm)?;

            Ok(zelf)
        }

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            unimplemented!("use slot_new")
        }
    }

    impl Initializer for NodeAst {
        type Args = FuncArgs;

        fn slot_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            let fields = zelf.get_attr("_fields", vm)?;
            let fields: Vec<PyStrRef> = fields.try_to_value(vm)?;
            let n_args = args.args.len();
            if n_args > fields.len() {
                return Err(vm.new_type_error(format!(
                    "{} constructor takes at most {} positional argument{}",
                    zelf.class().name(),
                    fields.len(),
                    if fields.len() == 1 { "" } else { "s" },
                )));
            }

            // Track which fields were set
            let mut set_fields = std::collections::HashSet::new();

            for (name, arg) in fields.iter().zip(args.args) {
                zelf.set_attr(name, arg, vm)?;
                set_fields.insert(name.as_str().to_string());
            }
            for (key, value) in args.kwargs {
                if let Some(pos) = fields.iter().position(|f| f.as_str() == key)
                    && pos < n_args
                {
                    return Err(vm.new_type_error(format!(
                        "{} got multiple values for argument '{}'",
                        zelf.class().name(),
                        key
                    )));
                }
                set_fields.insert(key.clone());
                zelf.set_attr(vm.ctx.intern_str(key), value, vm)?;
            }

            // Set default values only for built-in AST nodes (_field_types present).
            // Custom AST subclasses without _field_types do NOT get automatic defaults.
            let has_field_types = zelf
                .class()
                .get_attr(vm.ctx.intern_str("_field_types"))
                .is_some();
            if has_field_types {
                // ASDL list fields (type*) default to empty list,
                // optional/required fields default to None.
                // Fields that are always list-typed regardless of node class.
                const LIST_FIELDS: &[&str] = &[
                    "argtypes",
                    "bases",
                    "cases",
                    "comparators",
                    "decorator_list",
                    "defaults",
                    "elts",
                    "finalbody",
                    "generators",
                    "handlers",
                    "ifs",
                    "items",
                    "keys",
                    "kw_defaults",
                    "kwd_attrs",
                    "kwd_patterns",
                    "keywords",
                    "kwonlyargs",
                    "names",
                    "ops",
                    "patterns",
                    "posonlyargs",
                    "targets",
                    "type_ignores",
                    "type_params",
                    "values",
                ];

                let class_name = zelf.class().name().to_string();

                for field in &fields {
                    if !set_fields.contains(field.as_str()) {
                        let field_name = field.as_str();
                        // Some field names have different ASDL types depending on the node.
                        // For example, "args" is `expr*` in Call but `arguments` in Lambda.
                        // "body" and "orelse" are `stmt*` in most nodes but `expr` in IfExp.
                        let is_list_field = if field_name == "args" {
                            class_name == "Call" || class_name == "arguments"
                        } else if field_name == "body" || field_name == "orelse" {
                            !matches!(class_name.as_str(), "Lambda" | "Expression" | "IfExp")
                        } else {
                            LIST_FIELDS.contains(&field_name)
                        };

                        let default: PyObjectRef = if is_list_field {
                            vm.ctx.new_list(vec![]).into()
                        } else {
                            vm.ctx.none()
                        };
                        zelf.set_attr(vm.ctx.intern_str(field_name), default, vm)?;
                    }
                }

                // Special defaults that are not None or empty list
                if class_name == "ImportFrom" && !set_fields.contains("level") {
                    zelf.set_attr("level", vm.ctx.new_int(0), vm)?;
                }
            }

            Ok(())
        }

        fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            unreachable!("slot_init is defined")
        }
    }

    #[pyattr(name = "PyCF_ONLY_AST")]
    use super::PY_COMPILE_FLAG_AST_ONLY;

    #[pyattr(name = "PyCF_OPTIMIZED_AST")]
    use super::PY_CF_OPTIMIZED_AST;

    #[pyattr(name = "PyCF_TYPE_COMMENTS")]
    use super::PY_CF_TYPE_COMMENTS;

    pub(crate) fn module_exec(
        vm: &VirtualMachine,
        module: &Py<crate::builtins::PyModule>,
    ) -> PyResult<()> {
        __module_exec(vm, module);
        super::super::pyast::extend_module_nodes(vm, module);
        Ok(())
    }
}
