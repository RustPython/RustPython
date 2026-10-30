use crate::{
    Py, PyObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    builtins::{PyDict, PyDictRef, PyModule, PyStr, PyTuple, PyTupleRef, PyType},
    class::PyClassImpl,
    common::atomic::{AtomicUsize, Ordering},
    function::{Callee, FuncArgs, ItemDoc, KwArgs, OptionalArg, PyMethodDef, PyMethodFlags},
    protocol::{BufferDescriptor, BufferFlags, BufferMethods, PyBuffer},
    types::{AsBuffer, Constructor},
};

fn fastcall_args(args: &PyObject, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
    if vm.is_none(args) {
        Ok(Vec::new())
    } else if let Some(args) = args.downcast_ref::<PyTuple>() {
        Ok(args.as_slice().to_vec())
    } else {
        Err(vm.new_type_error("args must be None or a tuple"))
    }
}

fn check_keyword_names(names: &[PyObjectRef], vm: &VirtualMachine) -> PyResult<()> {
    if names
        .iter()
        .any(|name| name.downcast_ref::<PyStr>().is_none())
    {
        return Err(vm.new_type_error("keywords must be strings"));
    }
    Ok(())
}

// These adapters exercise the native VM call paths, not exported C-ABI symbols.
fn pyobject_vectorcall(vm: &VirtualMachine, args: FuncArgs, callee: Callee) -> PyResult {
    let (_module, callable, args, kwnames): (PyObjectRef, PyObjectRef, PyObjectRef, PyObjectRef) =
        args.bind_for(vm, callee)?;
    let args = fastcall_args(&args, vm)?;
    let kwnames = if vm.is_none(&kwnames) {
        None
    } else {
        Some(
            kwnames
                .downcast_ref::<PyTuple>()
                .ok_or_else(|| vm.new_type_error("kwnames must be None or a tuple"))?
                .as_slice(),
        )
    };
    let nargs = args
        .len()
        .checked_sub(kwnames.map_or(0, <[PyObjectRef]>::len))
        .ok_or_else(|| vm.new_value_error("kwnames longer than args"))?;
    if let Some(names) = kwnames {
        // The VM's internal vectorcall conversion assumes string keys.
        check_keyword_names(names, vm)?;
    }
    callable.vectorcall(args, nargs, kwnames, vm)
}

fn pyobject_fastcalldict(vm: &VirtualMachine, args: FuncArgs, callee: Callee) -> PyResult {
    let (_module, callable, args, kwargs): (PyObjectRef, PyObjectRef, PyObjectRef, PyObjectRef) =
        args.bind_for(vm, callee)?;
    let mut args = fastcall_args(&args, vm)?;
    let nargs = args.len();
    let mut names = Vec::new();
    if !vm.is_none(&kwargs) {
        let kwargs = kwargs
            .downcast_ref::<PyDict>()
            .ok_or_else(|| vm.new_type_error("kwnames must be None or a dict"))?;
        for (key, value) in kwargs.items_vec() {
            names.push(key);
            args.push(value);
        }
        check_keyword_names(&names, vm)?;
    }
    let kwnames = (!names.is_empty()).then_some(names.as_slice());
    callable.vectorcall(args, nargs, kwnames, vm)
}

fn pycfunction_call(vm: &VirtualMachine, args: FuncArgs, callee: Callee) -> PyResult {
    let (_module, callable, args, kwargs): (
        PyObjectRef,
        PyObjectRef,
        PyTupleRef,
        OptionalArg<PyDictRef>,
    ) = args.bind_for(vm, callee)?;
    let kwargs = match kwargs {
        OptionalArg::Missing => KwArgs::default(),
        OptionalArg::Present(kwargs) => kwargs
            .items_vec()
            .into_iter()
            .map(|(key, value)| {
                let name = key
                    .downcast_ref::<PyStr>()
                    .ok_or_else(|| vm.new_type_error("keywords must be strings"))?;
                Ok((name.as_wtf8().to_owned(), value))
            })
            .collect::<PyResult<KwArgs>>()?,
    };
    callable.call_with_args(FuncArgs::new(args.as_slice().to_vec(), kwargs), vm)
}

#[pyclass(module = false, name = "testBufType")]
#[derive(Debug, PyPayload)]
struct TestBuf {
    data: [u8; 4],
    references: AtomicUsize,
}

#[pyclass(with(Constructor, AsBuffer), flags(IMMUTABLETYPE))]
impl TestBuf {
    #[pygetset]
    fn references(zelf: &Py<Self>) -> usize {
        zelf.references.load(Ordering::Relaxed)
    }
}

impl Constructor for TestBuf {
    type Args = FuncArgs;

    fn py_new(_cls: &Py<PyType>, _args: FuncArgs, _vm: &VirtualMachine) -> PyResult<Self> {
        Ok(Self {
            data: *b"test",
            references: AtomicUsize::new(0),
        })
    }
}

static BUFFER_METHODS: BufferMethods = BufferMethods {
    obj_bytes: |buffer| buffer.obj_as::<TestBuf>().data.as_slice().into(),
    obj_bytes_mut: |_| unreachable!("testBuf exports a read-only buffer"),
    retain: |buffer| {
        buffer
            .obj_as::<TestBuf>()
            .references
            .fetch_add(1, Ordering::Relaxed);
    },
    release: |buffer| {
        let previous = buffer
            .obj_as::<TestBuf>()
            .references
            .fetch_sub(1, Ordering::Relaxed);
        debug_assert!(previous > 0);
    },
};

impl AsBuffer for TestBuf {
    const RELEASE_BUFFER: bool = true;

    fn slot_as_buffer(
        zelf: &PyObject,
        flags: BufferFlags,
        vm: &VirtualMachine,
    ) -> PyResult<PyBuffer> {
        flags.fill_info_check(true, vm)?;
        Self::as_buffer(zelf.try_downcast_ref::<Self>(vm)?, vm)
    }

    fn as_buffer(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
        Ok(PyBuffer::new(
            zelf.to_owned().into(),
            BufferDescriptor::simple(zelf.data.len(), true),
            &BUFFER_METHODS,
        ))
    }
}

pub(super) fn extend_module(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
    module.set_attr("testBuf", TestBuf::make_static_type(), vm)?;
    const METHODS: &[PyMethodDef] = &[
        PyMethodDef::new_raw_const(
            "pyobject_vectorcall",
            pyobject_vectorcall,
            PyMethodFlags::FASTCALL,
            ItemDoc::NONE,
        ),
        PyMethodDef::new_raw_const(
            "pyobject_fastcalldict",
            pyobject_fastcalldict,
            PyMethodFlags::FASTCALL,
            ItemDoc::NONE,
        ),
        PyMethodDef::new_raw_const(
            "pycfunction_call",
            pycfunction_call,
            PyMethodFlags::VARARGS,
            ItemDoc::NONE,
        ),
    ];
    for method in METHODS {
        let function = method.build_bound_function(&vm.ctx, module.to_owned().into());
        drop(
            function
                .module
                .store(Some(vm.ctx.new_str("_testcapi").into())),
        );
        module.set_attr(method.name, function, vm)?;
    }
    Ok(())
}
