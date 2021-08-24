pub(crate) use _contextvars::make_module;

#[pymodule]
mod _contextvars {
    use crate::builtins::PyTypeRef;
    use crate::pyobject::PyClassImpl;
    use crate::pyobject::PyContext;
    use crate::slots::PyTypeSlots;

    #[pyattr]
    #[pyclass(name = "Context")]
    #[derive(Debug, Default)]
    struct Context {}

    impl PyClassImpl for Context {
        const TP_FLAGS: crate::slots::PyTpFlags = crate::slots::PyTpFlags::DEFAULT;

        fn impl_extend_class(ctx: &PyContext, class: &PyTypeRef) {
            // TODO: RUSTPYTHON
        }

        fn extend_slots(slots: &mut PyTypeSlots) {
            // TODO: RUSTPYTHON
        }
    }

    #[pyattr]
    #[pyclass(name = "ContextVar")]
    #[derive(Debug, Default)]
    struct ContextVar {}

    impl PyClassImpl for ContextVar {
        const TP_FLAGS: crate::slots::PyTpFlags = crate::slots::PyTpFlags::DEFAULT;

        fn impl_extend_class(ctx: &PyContext, class: &PyTypeRef) {
            // TODO: RUSTPYTHON
        }

        fn extend_slots(slots: &mut PyTypeSlots) {
            // TODO: RUSTPYTHON
        }
    }

    #[pyattr]
    #[pyclass(name = "Token")]
    #[derive(Debug, Default)]
    struct ContextToken {}

    impl PyClassImpl for ContextToken {
        const TP_FLAGS: crate::slots::PyTpFlags = crate::slots::PyTpFlags::DEFAULT;

        fn impl_extend_class(ctx: &PyContext, class: &PyTypeRef) {
            // TODO: RUSTPYTHON
        }

        fn extend_slots(slots: &mut PyTypeSlots) {
            // TODO: RUSTPYTHON
        }
    }

    #[pyfunction]
    fn copy_context() {}
}
