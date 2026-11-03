//! Read-only resource protocols for explicitly registered frozen assets.

#[pymodule(sub, name = "_imp")]
pub(super) mod resources {
    use crate::{
        AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyStrRef},
        frozen_resources::{ResourceError, ResourceView},
        function::{FsPath, FuncArgs, OptionalArg, PosArgs},
        protocol::PyNumberMethods,
        types::AsNumber,
    };

    fn resource_error(error: ResourceError, vm: &VirtualMachine) -> PyBaseExceptionRef {
        let exception = match &error {
            ResourceError::InvalidPath(_) => return vm.new_value_error(error.to_string()),
            ResourceError::NotFound(_) => vm.ctx.exceptions.file_not_found_error,
            ResourceError::IsDirectory(_) => vm.ctx.exceptions.is_a_directory_error,
            ResourceError::NotDirectory(_) => vm.ctx.exceptions.not_a_directory_error,
            _ => return vm.new_runtime_error(error.to_string()),
        };
        vm.new_os_subtype_error(exception.to_owned(), None, error.to_string())
            .upcast()
    }

    fn join_resource(
        view: &ResourceView,
        path: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<ResourceView> {
        let path = FsPath::try_from_path_like(path, false, vm)?;
        let FsPath::Str(path) = path else {
            return Err(vm.new_type_error("resource paths must be str or os.PathLike[str]"));
        };
        let path = path
            .to_str()
            .ok_or_else(|| vm.new_value_error("resource paths must not contain surrogates"))?;
        view.join(path).map_err(|error| resource_error(error, vm))
    }

    fn open_binary(view: &ResourceView, vm: &VirtualMachine) -> PyResult {
        let bytes = view.bytes().map_err(|error| resource_error(error, vm))?;
        // Import lazily: installing the importer hook must not bootstrap _io.
        let io = vm.import("_io", 0)?;
        let buffer = io
            .get_attr("BytesIO", vm)?
            .call((vm.ctx.new_bytes(bytes.to_vec()),), vm)?;
        io.get_attr("BufferedReader", vm)?.call((buffer,), vm)
    }

    #[derive(FromArgs)]
    struct OpenArgs {
        #[pyarg(any, optional, py_default = "'r'")]
        mode: OptionalArg<PyStrRef>,
        #[pyarg(any, optional)]
        encoding: Option<PyObjectRef>,
        #[pyarg(any, optional)]
        errors: Option<PyObjectRef>,
        #[pyarg(any, optional)]
        newline: Option<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct ReadTextArgs {
        #[pyarg(any, optional)]
        encoding: Option<PyObjectRef>,
        #[pyarg(any, optional)]
        errors: Option<PyObjectRef>,
        #[pyarg(named, optional)]
        newline: Option<PyObjectRef>,
    }

    #[pyattr]
    #[pyclass(name = "_FrozenResourceReader", module = "_imp")]
    #[derive(Debug, PyPayload)]
    struct FrozenResourceReader {
        view: ResourceView,
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION))]
    impl FrozenResourceReader {
        #[pymethod]
        fn open_resource(zelf: &Py<Self>, resource: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            open_binary(&join_resource(&zelf.view, resource, vm)?, vm)
        }

        #[pymethod]
        fn resource_path(_zelf: &Py<Self>, resource: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            // Embedded assets have no filesystem path. importlib.resources.as_file
            // performs any requested temporary extraction through Traversable.
            Err(vm.new_exception(
                vm.ctx.exceptions.file_not_found_error.to_owned(),
                vec![resource],
            ))
        }

        #[pymethod]
        fn is_resource(
            zelf: &Py<Self>,
            resource: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<bool> {
            Ok(join_resource(&zelf.view, resource, vm)?.is_file())
        }

        #[pymethod]
        fn contents(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            let children = zelf
                .view
                .children()
                .map_err(|error| resource_error(error, vm))?;
            let names = children
                .iter()
                .map(|child| vm.ctx.new_str(child.name()).into())
                .collect();
            Ok(vm.ctx.new_tuple(names).as_object().get_iter(vm)?.into())
        }

        #[pymethod]
        fn files(zelf: &Py<Self>, vm: &VirtualMachine) -> PyRef<FrozenTraversable> {
            FrozenTraversable {
                view: zelf.view.clone(),
            }
            .into_ref(&vm.ctx)
        }
    }

    #[pyattr]
    #[pyclass(name = "_FrozenTraversable", module = "_imp")]
    #[derive(Debug, PyPayload)]
    struct FrozenTraversable {
        view: ResourceView,
    }

    #[pyclass(with(AsNumber), flags(DISALLOW_INSTANTIATION))]
    impl FrozenTraversable {
        #[pygetset]
        fn name(zelf: &Py<Self>) -> String {
            zelf.view.name().to_owned()
        }

        #[pymethod]
        fn iterdir(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            let children = zelf
                .view
                .children()
                .map_err(|error| resource_error(error, vm))?;
            let children = children
                .into_iter()
                .map(|view| Self { view }.into_ref(&vm.ctx).into())
                .collect();
            Ok(vm.ctx.new_tuple(children).as_object().get_iter(vm)?.into())
        }

        #[pymethod]
        fn is_dir(zelf: &Py<Self>) -> bool {
            zelf.view.is_dir()
        }

        #[pymethod]
        fn is_file(zelf: &Py<Self>) -> bool {
            zelf.view.is_file()
        }

        #[pymethod]
        fn joinpath(
            zelf: &Py<Self>,
            descendants: PosArgs<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let mut view = zelf.view.clone();
            for descendant in descendants {
                view = join_resource(&view, descendant, vm)?;
            }
            Ok(Self { view }.into_ref(&vm.ctx))
        }

        #[pymethod]
        fn open(zelf: &Py<Self>, args: OpenArgs, vm: &VirtualMachine) -> PyResult {
            let mode = match &args.mode {
                OptionalArg::Present(mode) => mode.as_bytes(),
                OptionalArg::Missing => b"r",
            };
            match mode {
                b"rb" => {
                    if args.encoding.is_some() || args.errors.is_some() || args.newline.is_some() {
                        return Err(vm.new_value_error(
                            "binary mode does not accept encoding, errors or newline",
                        ));
                    }
                    open_binary(&zelf.view, vm)
                }
                b"r" => {
                    let buffer = open_binary(&zelf.view, vm)?;
                    let mut wrapper_args = FuncArgs::from(vec![buffer]);
                    for (name, value) in [
                        ("encoding", args.encoding),
                        ("errors", args.errors),
                        ("newline", args.newline),
                    ] {
                        if let Some(value) = value {
                            wrapper_args.kwargs.insert(name.into(), value);
                        }
                    }
                    vm.import("_io", 0)?
                        .get_attr("TextIOWrapper", vm)?
                        .call(wrapper_args, vm)
                }
                _ => Err(vm.new_value_error("frozen resources support only 'r' and 'rb' modes")),
            }
        }

        #[pymethod]
        fn read_bytes(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult {
            let bytes = zelf
                .view
                .bytes()
                .map_err(|error| resource_error(error, vm))?;
            Ok(vm.ctx.new_bytes(bytes.to_vec()).into())
        }

        #[pymethod]
        fn read_text(zelf: &Py<Self>, args: ReadTextArgs, vm: &VirtualMachine) -> PyResult {
            let stream = Self::open(
                zelf,
                OpenArgs {
                    mode: OptionalArg::Missing,
                    encoding: args.encoding,
                    errors: args.errors,
                    newline: args.newline,
                },
                vm,
            )?;
            let result = vm.call_method(&stream, "read", ());
            let close = vm.call_method(&stream, "close", ());
            match (result, close) {
                (Err(read_error), Err(close_error)) => {
                    close_error.set_context(Some(read_error));
                    Err(close_error)
                }
                (Err(error), Ok(_)) | (Ok(_), Err(error)) => Err(error),
                (Ok(contents), Ok(_)) => Ok(contents),
            }
        }
    }

    impl AsNumber for FrozenTraversable {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                true_divide: Some(|left, right, vm| {
                    let Some(path) = left.downcast_ref::<FrozenTraversable>() else {
                        return Ok(vm.ctx.not_implemented());
                    };
                    let view = join_resource(&path.view, right.to_owned(), vm)?;
                    Ok(FrozenTraversable { view }.into_ref(&vm.ctx).into())
                }),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    #[pyfunction]
    fn _get_frozen_resource_reader(
        fullname: PyStrRef,
        vm: &VirtualMachine,
    ) -> Option<PyRef<FrozenResourceReader>> {
        let name = fullname.to_str()?;
        super::super::find_frozen(name, vm).ok()?;
        // Resources belong to an explicit exact-fullname registration. Resolving
        // an unbound alias to another name would bypass registration ownership.
        let view = vm.state.frozen.resource_view(name)?;
        Some(FrozenResourceReader { view }.into_ref(&vm.ctx))
    }
}
