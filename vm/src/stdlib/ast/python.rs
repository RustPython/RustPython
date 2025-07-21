use super::{PY_CF_OPTIMIZED_AST, PY_CF_TYPE_COMMENTS, PY_COMPILE_FLAG_AST_ONLY};

#[pymodule]
pub(crate) mod _ast {
    use crate::{
        AsObject, Context, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{PyStrRef, PyTupleRef, PyTypeRef},
        function::FuncArgs,
        types::Constructor,
    };
    #[pyattr]
    #[pyclass(module = "_ast", name = "AST")]
    #[derive(Debug, PyPayload)]
    pub(crate) struct NodeAst;

    #[pyclass(with(Constructor), flags(BASETYPE, HAS_DICT))]
    impl NodeAst {
        #[pyslot]
        #[pymethod]
        fn __init__(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
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
            for (name, arg) in fields.iter().zip(args.args) {
                zelf.set_attr(name, arg, vm)?;
            }
            for (key, value) in args.kwargs {
                if let Some(pos) = fields.iter().position(|f| f.as_str() == key) {
                    if pos < n_args {
                        return Err(vm.new_type_error(format!(
                            "{} got multiple values for argument '{}'",
                            zelf.class().name(),
                            key
                        )));
                    }
                }
                zelf.set_attr(vm.ctx.intern_str(key), value, vm)?;
            }
            Ok(())
        }

        #[pyattr(name = "_fields")]
        fn fields(ctx: &Context) -> PyTupleRef {
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
            NodeAst::__init__(zelf.clone(), args, vm)?;

            Ok(zelf)
        }

        fn py_new(_cls: PyTypeRef, _args: Self::Args, _vm: &VirtualMachine) -> PyResult {
            unreachable!("slow_new is implemented");
        }
    }

    #[pyattr(name = "PyCF_ONLY_AST")]
    use super::PY_COMPILE_FLAG_AST_ONLY;

    #[pyattr(name = "PyCF_OPTIMIZED_AST")]
    use super::PY_CF_OPTIMIZED_AST;

    #[pyattr(name = "PyCF_TYPE_COMMENTS")]
    use super::PY_CF_TYPE_COMMENTS;
}
