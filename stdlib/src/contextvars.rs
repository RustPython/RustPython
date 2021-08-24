pub(crate) use _contextvars::make_module;

#[pymodule]
mod _contextvars {
    #[pyattr]
    #[pyclass(name = "Context")]
    #[derive(Debug, Default)]
    struct Context {}

    #[pyimpl]
    impl Context {}

    #[pyattr]
    #[pyclass(name = "ContextVar")]
    #[derive(Debug, Default)]
    struct ContextVar {}

    #[pyimpl]
    impl ContextVar {}

    #[pyattr]
    #[pyclass(name = "Token")]
    #[derive(Debug, Default)]
    struct ContextToken {}

    #[pyimpl]
    impl ContextToken {}

    #[pyfunction]
    fn copy_context() {}
}
