use crate::{
    Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{PyStrRef, PyType, PyTypeRef, type_::PyAttributes},
    class::add_operators,
    function::FuncArgs,
    object::{Traverse, TraverseFn},
    protocol::PySequence,
    types::{PyTypeFlags, PyTypeSlots, fn_addr},
};

// Like RustPython's tuple payload, the element storage is separately allocated
// and independent of the instance's managed dictionary and weakref cells.
#[pyclass(module = "_testcapi", name = "HeapCCollection", traverse = "manual")]
#[derive(Debug)]
struct HeapCCollection {
    items: Box<[PyObjectRef]>,
}

// This payload has no process-global Python class. Its allocation and typed
// access use the per-module class marker and the payload ID, respectively.
impl PyPayload for HeapCCollection {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.object_type
    }

    fn try_downcast_from(obj: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        if obj.downcastable::<Self>() {
            Ok(())
        } else {
            Err(vm.new_type_error("expected a HeapCCollection instance"))
        }
    }
}

struct HeapCCollectionType;

// SAFETY: every owned element is visited exactly once. Clearing takes ownership
// of all elements before the collector releases any child references.
unsafe impl Traverse for HeapCCollection {
    fn traverse(&self, tracer: &mut TraverseFn<'_>) {
        self.items.traverse(tracer);
    }

    fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
        out.extend(core::mem::take(&mut self.items).into_vec());
    }
}

impl HeapCCollection {
    const BASICSIZE: usize = crate::object::payload_offset::<Self>() + core::mem::size_of::<Self>();

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // Check the physical base chain, rather than a customizable MRO. Only
        // classes created by this module can supply the native allocation root.
        let mut base = Some(cls.clone());
        let mut native_base = false;
        while let Some(current) = base {
            if current.has_type_data::<HeapCCollectionType>() {
                native_base = true;
                break;
            }
            base = current.base.load_owned();
        }
        if !native_base
            || cls.slots.basicsize != Self::BASICSIZE
            || cls.slots.itemsize != core::mem::size_of::<PyObjectRef>()
        {
            return Err(vm.new_type_error(format!(
                "cannot create '{}' with HeapCCollection's native layout",
                cls.name()
            )));
        }
        // The C fixture stores each positional argument and ignores keywords.
        let payload = Self {
            items: args.args.into_boxed_slice(),
        };
        let dict = cls
            .slots
            .flags
            .has_feature(PyTypeFlags::HAS_DICT)
            .then(|| vm.ctx.new_dict());
        Ok(PyRef::new_ref(payload, cls, dict).into())
    }

    fn length(sequence: PySequence<'_>, vm: &VirtualMachine) -> PyResult<usize> {
        let collection = sequence
            .obj
            .downcast_ref::<Self>()
            .ok_or_else(|| vm.new_type_error("expected a HeapCCollection instance"))?;
        Ok(collection.items.len())
    }

    fn item(sequence: PySequence<'_>, index: isize, vm: &VirtualMachine) -> PyResult {
        let collection = sequence
            .obj
            .downcast_ref::<Self>()
            .ok_or_else(|| vm.new_type_error("expected a HeapCCollection instance"))?;
        let index = if index < 0 {
            index + collection.items.len() as isize
        } else {
            index
        };
        usize::try_from(index)
            .ok()
            .and_then(|index| collection.items.get(index))
            .cloned()
            .ok_or_else(|| vm.new_index_error(format!("index {index} out of range")))
    }
}

pub(crate) fn heap_c_collection(vm: &VirtualMachine) -> PyTypeRef {
    let slots = PyTypeSlots {
        flags: crate::types::AtomicPyTypeFlags::from_plain(PyTypeFlags::from_slice(&[
            PyTypeFlags::BASETYPE,
            PyTypeFlags::HAVE_GC,
        ])),
        basicsize: HeapCCollection::BASICSIZE,
        itemsize: core::mem::size_of::<PyObjectRef>(),
        ..PyTypeSlots::default()
    };
    let class = vm.ctx.new_class(
        Some("_testcapi"),
        "HeapCCollection",
        vm.ctx.types.object_type.to_owned(),
        slots,
    );
    class
        .init_type_data(HeapCCollectionType)
        .expect("new HeapCCollection type has no existing type data");
    class.slots.new.store(Some(HeapCCollection::slot_new));
    class
        .slots
        .as_sequence
        .length
        .store(Some(HeapCCollection::length));
    class
        .slots
        .as_sequence
        .item
        .store(Some(HeapCCollection::item));
    let new = Context::genesis()
        .slot_new_wrapper
        .build_bound_function(&vm.ctx, class.clone().into());
    class.set_attr(identifier!(vm, __new__), new.into());
    class.set_attr(identifier!(vm, __doc__), vm.ctx.none());
    add_operators(&class, &vm.ctx, &[]);
    class
}

pub(crate) fn heap_ctype_metaclass_null_new(vm: &VirtualMachine) -> PyTypeRef {
    let slots = PyTypeSlots {
        flags: crate::types::AtomicPyTypeFlags::from_plain(PyTypeFlags::from_slice(&[
            PyTypeFlags::DISALLOW_INSTANTIATION,
        ])),
        itemsize: vm.ctx.types.type_type.slots.itemsize,
        ..PyTypeSlots::default()
    };
    vm.ctx.new_class(
        Some("_testcapi"),
        "HeapCTypeMetaclassNullNew",
        vm.ctx.types.type_type.to_owned(),
        slots,
    )
}

pub(crate) fn pytype_fromspec_meta(meta: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyTypeRef> {
    let meta = meta.downcast::<PyType>().map_err(|_| {
        vm.new_type_error("pytype_fromspec_meta: must be invoked with a type argument!")
    })?;
    if !meta.fast_issubclass(vm.ctx.types.type_type) {
        return Err(vm.new_type_error(format!(
            "Metaclass '{}' is not a subclass of 'type'.",
            meta.name()
        )));
    }
    if meta.slots.new.load().is_some_and(|new| {
        Some(fn_addr(new)) != vm.ctx.types.type_type.slots.new.load().map(fn_addr)
    }) {
        return Err(vm.new_type_error("Metaclasses with custom tp_new are not supported."));
    }
    let mut attrs = PyAttributes::default();
    attrs.insert(
        identifier!(vm, __module__),
        vm.ctx.new_str("_testcapi").into(),
    );
    PyType::new_heap(
        "HeapCTypeViaMetaclass",
        vec![vm.ctx.types.object_type.to_owned()],
        attrs,
        PyTypeSlots {
            flags: crate::types::AtomicPyTypeFlags::from_plain(PyTypeFlags::HEAP_TYPE),
            ..PyTypeSlots::default()
        },
        meta,
        &vm.ctx,
    )
    .map_err(|message| vm.new_type_error(message))
}

pub(crate) fn bad_get(
    descriptor: PyObjectRef,
    _object: PyObjectRef,
    class: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<PyStrRef> {
    // Keep an owned reference across the call, which may remove the descriptor
    // from its class and trigger collection of otherwise unreachable objects.
    drop(class.call((), vm)?);
    descriptor.repr(vm)
}
