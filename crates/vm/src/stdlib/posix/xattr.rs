// spell-checker:disable

#[pymodule(sub, name = "posix")]
pub(super) mod _xattr {
    use crate::{
        PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyBytesRef, PyListRef},
        convert::TryFromObject,
        exceptions::ToOSErrorBuilder,
        function::{ArgBytesLike, ArgIntoBool},
        ospath::PathConverter,
        stdlib::os::warn_if_bool_fd,
    };
    use alloc::ffi::CString;
    use rustpython_host_env::{os::ffi::OsStringExt, posix::xattr::Target};
    use std::{ffi::OsString, io};

    #[pyattr]
    use rustpython_host_env::posix::xattr::XATTR_SIZE_MAX;

    const GET: usize = 0;
    const SET: usize = 1;
    const REMOVE: usize = 2;
    const LIST: usize = 3;
    const NAME: [&str; 4] = ["getxattr", "setxattr", "removexattr", "listxattr"];

    enum Location {
        Path(CString),
        Fd(i32),
    }

    struct Path<const OP: usize> {
        location: Location,
        original: PyObjectRef,
    }

    impl<const OP: usize> TryFromObject for Path<OP> {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            let converter = PathConverter::new().function(NAME[OP]);

            let to_path = |path: crate::ospath::OsPath| -> PyResult<Self> {
                let original = path.filename(vm)?;
                let location = Location::Path(path.into_cstring(vm)?);
                Ok(Self { location, original })
            };

            let obj = match converter.try_match_str_bytes(obj, vm)? {
                Ok(path) => return to_path(path),
                Err(obj) => obj,
            };
            if let Some(fd) = obj.try_index_opt(vm) {
                warn_if_bool_fd(&obj, vm)?;
                let fd = fd?;
                let fd = i32::try_from(fd.as_bigint()).map_err(|_| {
                    let message = if fd.as_bigint().sign() == malachite_bigint::Sign::Minus {
                        "fd is less than minimum"
                    } else {
                        "fd is greater than maximum"
                    };
                    vm.new_overflow_error(message)
                })?;
                Ok(Self {
                    location: Location::Fd(fd),
                    original: obj,
                })
            } else {
                to_path(converter.try_pathlike_with_nullable(obj, true, OP == LIST, vm)?)
            }
        }
    }

    impl<const OP: usize> Path<OP> {
        fn cwd(vm: &VirtualMachine) -> Self {
            Self {
                location: Location::Path(c".".into()),
                original: vm.ctx.none(),
            }
        }

        fn target(&self, follow_symlinks: bool, vm: &VirtualMachine) -> PyResult<Target<'_>> {
            match &self.location {
                Location::Path(path) if follow_symlinks => Ok(Target::Follow(path)),
                Location::Path(path) => Ok(Target::NoFollow(path)),
                Location::Fd(fd) if follow_symlinks => Ok(Target::Fd(*fd)),
                Location::Fd(_) => Err(vm.new_value_error(format!(
                    "{}: cannot use fd and follow_symlinks together",
                    NAME[OP],
                ))),
            }
        }

        fn error(
            &self,
            err: io::Error,
            vm: &VirtualMachine,
        ) -> crate::builtins::PyBaseExceptionRef {
            if err.kind() == io::ErrorKind::OutOfMemory && err.raw_os_error().is_none() {
                vm.no_memory_error()
            } else {
                err.to_os_error_builder(vm)
                    .filename(self.original.clone())
                    .build(vm)
                    .upcast()
            }
        }
    }

    struct Attribute<const OP: usize>(CString, PyObjectRef);

    impl<const OP: usize> TryFromObject for Attribute<OP> {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            let path = PathConverter::new()
                .function(NAME[OP])
                .argument("attribute")
                .try_path(obj, vm)?;
            let original = path.filename(vm)?;
            Ok(Self(path.into_cstring(vm)?, original))
        }
    }

    #[derive(FromArgs)]
    struct AttributeArgs<const OP: usize> {
        #[pyarg(any)]
        path: Path<OP>,
        #[pyarg(any)]
        attribute: Attribute<OP>,
        #[pyarg(named, default = ArgIntoBool::TRUE)]
        follow_symlinks: ArgIntoBool,
    }

    #[pyfunction]
    fn getxattr(args: AttributeArgs<GET>, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        let AttributeArgs {
            path,
            attribute,
            follow_symlinks,
        } = args;
        let target = path.target(follow_symlinks.into_bool(), vm)?;
        vm.audit("os.getxattr", || (path.original.clone(), attribute.1))?;
        vm.allow_threads(|| target.get(&attribute.0))
            .map(|value| vm.ctx.new_bytes(value))
            .map_err(|err| path.error(err, vm))
    }

    #[derive(FromArgs)]
    struct SetArgs {
        #[pyarg(any)]
        path: Path<SET>,
        #[pyarg(any)]
        attribute: Attribute<SET>,
        #[pyarg(any)]
        value: ArgBytesLike,
        #[pyarg(any, default = 0)]
        flags: i32,
        #[pyarg(named, default = ArgIntoBool::TRUE)]
        follow_symlinks: ArgIntoBool,
    }

    #[pyfunction]
    fn setxattr(args: SetArgs, vm: &VirtualMachine) -> PyResult<()> {
        let SetArgs {
            path,
            attribute,
            value,
            flags,
            follow_symlinks,
        } = args;
        let target = path.target(follow_symlinks.into_bool(), vm)?;
        vm.audit("os.setxattr", || {
            (
                path.original.clone(),
                attribute.1,
                value.with_ref(|buf| vm.ctx.new_bytes(buf.to_vec())),
                flags,
            )
        })?;
        let value = value.borrow_buf_unlocked(vm)?;
        vm.allow_threads(|| target.set(&attribute.0, &value, flags))
            .map_err(|err| path.error(err, vm))
    }

    #[pyfunction]
    fn removexattr(args: AttributeArgs<REMOVE>, vm: &VirtualMachine) -> PyResult<()> {
        let AttributeArgs {
            path,
            attribute,
            follow_symlinks,
        } = args;
        let target = path.target(follow_symlinks.into_bool(), vm)?;
        vm.audit("os.removexattr", || (path.original.clone(), attribute.1))?;
        vm.allow_threads(|| target.remove(&attribute.0))
            .map_err(|err| path.error(err, vm))
    }

    #[derive(FromArgs)]
    struct ListArgs {
        #[pyarg(any, default = None)]
        path: Option<Path<LIST>>,
        #[pyarg(named, default = ArgIntoBool::TRUE)]
        follow_symlinks: ArgIntoBool,
    }

    #[pyfunction]
    fn listxattr(args: ListArgs, vm: &VirtualMachine) -> PyResult<PyListRef> {
        let path = args.path.unwrap_or_else(|| Path::cwd(vm));
        let target = path.target(args.follow_symlinks.into_bool(), vm)?;
        vm.audit("os.listxattr", || (path.original.clone(),))?;
        let attrs = vm
            .allow_threads(|| target.list())
            .map_err(|err| path.error(err, vm))?;
        // The OS returns a sequence of NUL-terminated names, not UTF-8.
        let names = attrs
            .split_inclusive(|b| *b == 0)
            .filter_map(|name| name.strip_suffix(&[0]))
            .map(|name| {
                vm.fsdecode(OsString::from_vec(name.to_vec()))
                    .map(Into::into)
            })
            .collect::<PyResult<Vec<_>>>()?;
        Ok(vm.ctx.new_list(names))
    }
}
