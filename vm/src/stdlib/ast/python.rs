use super::PY_COMPILE_FLAG_AST_ONLY;

#[pymodule]
pub(crate) mod _ast {
    use crate::{
        AsObject, Context, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{PyStrRef, PyTupleRef},
        function::FuncArgs,
    };
    #[pyattr]
    #[pyclass(module = "_ast", name = "AST")]
    #[derive(Debug, PyPayload)]
    pub(crate) struct NodeAst;

    #[pyclass(flags(BASETYPE, HAS_DICT))]
    impl NodeAst {
        #[pyslot]
        #[pymethod(magic)]
        fn init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
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

    #[pyattr(name = "PyCF_ONLY_AST")]
    use super::PY_COMPILE_FLAG_AST_ONLY;
}
