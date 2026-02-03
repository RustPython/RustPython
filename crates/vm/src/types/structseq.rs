use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine, atomic_func,
    builtins::{PyBaseExceptionRef, PyStrRef, PyTuple, PyTupleRef, PyType, PyTypeRef},
    class::{PyClassImpl, StaticType},
    function::{Either, FuncArgs, PyComparisonValue, PyMethodDef, PyMethodFlags},
    iter::PyExactSizeIterator,
    protocol::{PyMappingMethods, PySequenceMethods},
    sliceable::{SequenceIndex, SliceableSequenceOp},
    types::PyComparisonOp,
    vm::Context,
};
use std::sync::LazyLock;

const DEFAULT_STRUCTSEQ_REDUCE: PyMethodDef = PyMethodDef::new_const(
    "__reduce__",
    |zelf: PyRef<PyTuple>, vm: &VirtualMachine| -> PyTupleRef {
        vm.new_tuple((zelf.class().to_owned(), (vm.ctx.new_tuple(zelf.to_vec()),)))
    },
    PyMethodFlags::METHOD,
    None,
);

/// Create a new struct sequence instance from a sequence.
///
/// The class must have `n_sequence_fields` and `n_fields` attributes set
/// (done automatically by `PyStructSequence::extend_pyclass`).
pub fn struct_sequence_new(cls: PyTypeRef, seq: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    // = structseq_new

    #[cold]
    fn length_error(
        tp_name: &str,
        min_len: usize,
        max_len: usize,
        len: usize,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        if min_len == max_len {
            vm.new_type_error(format!(
                "{tp_name}() takes a {min_len}-sequence ({len}-sequence given)"
            ))
        } else if len < min_len {
            vm.new_type_error(format!(
                "{tp_name}() takes an at least {min_len}-sequence ({len}-sequence given)"
            ))
        } else {
            vm.new_type_error(format!(
                "{tp_name}() takes an at most {max_len}-sequence ({len}-sequence given)"
            ))
        }
    }

    let min_len: usize = cls
        .get_attr(identifier!(vm.ctx, n_sequence_fields))
        .ok_or_else(|| vm.new_type_error("missing n_sequence_fields attribute"))?
        .try_into_value(vm)?;
    let max_len: usize = cls
        .get_attr(identifier!(vm.ctx, n_fields))
        .ok_or_else(|| vm.new_type_error("missing n_fields attribute"))?
        .try_into_value(vm)?;

    let seq: Vec<PyObjectRef> = seq.try_into_value(vm)?;
    let len = seq.len();

    if len < min_len || len > max_len {
        return Err(length_error(&cls.slot_name(), min_len, max_len, len, vm));
    }

    // Copy items and pad with None
    let mut items = seq;
    items.resize_with(max_len, || vm.ctx.none());

    PyTuple::new_unchecked(items.into_boxed_slice())
        .into_ref_with_type(vm, cls)
        .map(Into::into)
}

fn get_visible_len(obj: &PyObject, vm: &VirtualMachine) -> PyResult<usize> {
    obj.class()
        .get_attr(identifier!(vm.ctx, n_sequence_fields))
        .ok_or_else(|| vm.new_type_error("missing n_sequence_fields"))?
        .try_into_value(vm)
}

/// Sequence methods for struct sequences.
/// Uses n_sequence_fields to determine visible length.
static STRUCT_SEQUENCE_AS_SEQUENCE: LazyLock<PySequenceMethods> =
    LazyLock::new(|| PySequenceMethods {
        length: atomic_func!(|seq, vm| get_visible_len(seq.obj, vm)),
        concat: atomic_func!(|seq, other, vm| {
            // Convert to visible-only tuple, then use regular tuple concat
            let n_seq = get_visible_len(seq.obj, vm)?;
            let tuple = seq.obj.downcast_ref::<PyTuple>().unwrap();
            let visible: Vec<_> = tuple.iter().take(n_seq).cloned().collect();
            let visible_tuple = PyTuple::new_ref(visible, &vm.ctx);
            // Use tuple's concat implementation
            visible_tuple
                .as_object()
                .sequence_unchecked()
                .concat(other, vm)
        }),
        repeat: atomic_func!(|seq, n, vm| {
            // Convert to visible-only tuple, then use regular tuple repeat
            let n_seq = get_visible_len(seq.obj, vm)?;
            let tuple = seq.obj.downcast_ref::<PyTuple>().unwrap();
            let visible: Vec<_> = tuple.iter().take(n_seq).cloned().collect();
            let visible_tuple = PyTuple::new_ref(visible, &vm.ctx);
            // Use tuple's repeat implementation
            visible_tuple.as_object().sequence_unchecked().repeat(n, vm)
        }),
        item: atomic_func!(|seq, i, vm| {
            let n_seq = get_visible_len(seq.obj, vm)?;
            let tuple = seq.obj.downcast_ref::<PyTuple>().unwrap();
            let idx = if i < 0 {
                let pos_i = n_seq as isize + i;
                if pos_i < 0 {
                    return Err(vm.new_index_error("tuple index out of range"));
                }
                pos_i as usize
            } else {
                i as usize
            };
            if idx >= n_seq {
                return Err(vm.new_index_error("tuple index out of range"));
            }
            Ok(tuple[idx].clone())
        }),
        contains: atomic_func!(|seq, needle, vm| {
            let n_seq = get_visible_len(seq.obj, vm)?;
            let tuple = seq.obj.downcast_ref::<PyTuple>().unwrap();
            for item in tuple.iter().take(n_seq) {
                if item.rich_compare_bool(needle, PyComparisonOp::Eq, vm)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }),
        ..PySequenceMethods::NOT_IMPLEMENTED
    });

/// Mapping methods for struct sequences.
/// Handles subscript (indexing) with visible length bounds.
static STRUCT_SEQUENCE_AS_MAPPING: LazyLock<PyMappingMethods> =
    LazyLock::new(|| PyMappingMethods {
        length: atomic_func!(|mapping, vm| get_visible_len(mapping.obj, vm)),
        subscript: atomic_func!(|mapping, needle, vm| {
            let n_seq = get_visible_len(mapping.obj, vm)?;
            let tuple = mapping.obj.downcast_ref::<PyTuple>().unwrap();
            let visible_elements = &tuple.as_slice()[..n_seq];

            match SequenceIndex::try_from_borrowed_object(vm, needle, "tuple")? {
                SequenceIndex::Int(i) => visible_elements.getitem_by_index(vm, i),
                SequenceIndex::Slice(slice) => visible_elements
                    .getitem_by_slice(vm, slice)
                    .map(|x| vm.ctx.new_tuple(x).into()),
            }
        }),
        ..PyMappingMethods::NOT_IMPLEMENTED
    });

/// Trait for Data structs that back a PyStructSequence.
///
/// This trait is implemented by `#[pystruct_sequence_data]` on the Data struct.
/// It provides field information, tuple conversion, and element parsing.
pub trait PyStructSequenceData: Sized {
    /// Names of required fields (in order). Shown in repr.
    const REQUIRED_FIELD_NAMES: &'static [&'static str];

    /// Names of optional/skipped fields (in order, after required fields).
    const OPTIONAL_FIELD_NAMES: &'static [&'static str];

    /// Number of unnamed fields (visible but index-only access).
    const UNNAMED_FIELDS_LEN: usize = 0;

    /// Convert this Data struct into a PyTuple.
    fn into_tuple(self, vm: &VirtualMachine) -> PyTuple;

    /// Construct this Data struct from tuple elements.
    /// Default implementation returns an error.
    /// Override with `#[pystruct_sequence_data(try_from_object)]` to enable.
    fn try_from_elements(_elements: Vec<PyObjectRef>, vm: &VirtualMachine) -> PyResult<Self> {
        Err(vm.new_type_error("This struct sequence does not support construction from elements"))
    }
}

/// Trait for Python struct sequence types.
///
/// This trait is implemented by the `#[pystruct_sequence]` macro on the Python type struct.
/// It connects to the Data struct and provides Python-level functionality.
#[pyclass]
pub trait PyStructSequence: StaticType + PyClassImpl + Sized + 'static {
    /// The Data struct that provides field definitions.
    type Data: PyStructSequenceData;

    /// Convert a Data struct into a PyStructSequence instance.
    fn from_data(data: Self::Data, vm: &VirtualMachine) -> PyTupleRef {
        let tuple =
            <Self::Data as ::rustpython_vm::types::PyStructSequenceData>::into_tuple(data, vm);
        let typ = Self::static_type();
        tuple
            .into_ref_with_type(vm, typ.to_owned())
            .expect("Every PyStructSequence must be a valid tuple. This is a RustPython bug.")
    }

    #[pyslot]
    fn slot_repr(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let zelf = zelf
            .downcast_ref::<PyTuple>()
            .ok_or_else(|| vm.new_type_error("unexpected payload for __repr__"))?;

        let field_names = Self::Data::REQUIRED_FIELD_NAMES;
        let format_field = |(value, name): (&PyObject, _)| {
            let s = value.repr(vm)?;
            Ok(format!("{name}={s}"))
        };
        let (body, suffix) =
            if let Some(_guard) = rustpython_vm::recursion::ReprGuard::enter(vm, zelf.as_ref()) {
                let fields: PyResult<Vec<_>> = zelf
                    .iter()
                    .map(|value| value.as_ref())
                    .zip(field_names.iter().copied())
                    .map(format_field)
                    .collect();
                (fields?.join(", "), "")
            } else {
                (String::new(), "...")
            };
        let repr_str = format!("{}({}{})", Self::TP_NAME, body, suffix);
        Ok(vm.ctx.new_str(repr_str))
    }

    #[pymethod]
    fn __repr__(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        Self::slot_repr(&zelf, vm)
    }

    #[pymethod]
    fn __replace__(zelf: PyRef<PyTuple>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        if !args.args.is_empty() {
            return Err(vm.new_type_error("__replace__() takes no positional arguments".to_owned()));
        }

        if Self::Data::UNNAMED_FIELDS_LEN > 0 {
            return Err(vm.new_type_error(format!(
                "__replace__() is not supported for {} because it has unnamed field(s)",
                zelf.class().slot_name()
            )));
        }

        let n_fields =
            Self::Data::REQUIRED_FIELD_NAMES.len() + Self::Data::OPTIONAL_FIELD_NAMES.len();
        let mut items: Vec<PyObjectRef> = zelf.as_slice()[..n_fields].to_vec();

        let mut kwargs = args.kwargs.clone();

        // Replace fields from kwargs
        let all_field_names: Vec<&str> = Self::Data::REQUIRED_FIELD_NAMES
            .iter()
            .chain(Self::Data::OPTIONAL_FIELD_NAMES.iter())
            .copied()
            .collect();
        for (i, &name) in all_field_names.iter().enumerate() {
            if let Some(val) = kwargs.shift_remove(name) {
                items[i] = val;
            }
        }

        // Check for unexpected keyword arguments
        if !kwargs.is_empty() {
            let names: Vec<&str> = kwargs.keys().map(|k| k.as_str()).collect();
            return Err(vm.new_type_error(format!("Got unexpected field name(s): {:?}", names)));
        }

        PyTuple::new_unchecked(items.into_boxed_slice())
            .into_ref_with_type(vm, zelf.class().to_owned())
            .map(Into::into)
    }

    #[pymethod]
    fn __getitem__(zelf: PyRef<PyTuple>, needle: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let n_seq = get_visible_len(zelf.as_ref(), vm)?;
        let visible_elements = &zelf.as_slice()[..n_seq];

        match SequenceIndex::try_from_borrowed_object(vm, &needle, "tuple")? {
            SequenceIndex::Int(i) => visible_elements.getitem_by_index(vm, i),
            SequenceIndex::Slice(slice) => visible_elements
                .getitem_by_slice(vm, slice)
                .map(|x| vm.ctx.new_tuple(x).into()),
        }
    }

    #[extend_class]
    fn extend_pyclass(ctx: &Context, class: &'static Py<PyType>) {
        // Getters for named visible fields (indices 0 to REQUIRED_FIELD_NAMES.len() - 1)
        for (i, &name) in Self::Data::REQUIRED_FIELD_NAMES.iter().enumerate() {
            // cast i to a u8 so there's less to store in the getter closure.
            // Hopefully there's not struct sequences with >=256 elements :P
            let i = i as u8;
            class.set_attr(
                ctx.intern_str(name),
                ctx.new_readonly_getset(name, class, move |zelf: &PyTuple| {
                    zelf[i as usize].to_owned()
                })
                .into(),
            );
        }

        // Getters for hidden/skipped fields (indices after visible fields)
        let visible_count = Self::Data::REQUIRED_FIELD_NAMES.len() + Self::Data::UNNAMED_FIELDS_LEN;
        for (i, &name) in Self::Data::OPTIONAL_FIELD_NAMES.iter().enumerate() {
            let idx = (visible_count + i) as u8;
            class.set_attr(
                ctx.intern_str(name),
                ctx.new_readonly_getset(name, class, move |zelf: &PyTuple| {
                    zelf[idx as usize].to_owned()
                })
                .into(),
            );
        }

        class.set_attr(
            identifier!(ctx, __match_args__),
            ctx.new_tuple(
                Self::Data::REQUIRED_FIELD_NAMES
                    .iter()
                    .map(|&name| ctx.new_str(name).into())
                    .collect::<Vec<_>>(),
            )
            .into(),
        );

        // special fields:
        // n_sequence_fields = visible fields (named + unnamed)
        // n_fields = all fields (visible + hidden/skipped)
        // n_unnamed_fields
        let n_unnamed_fields = Self::Data::UNNAMED_FIELDS_LEN;
        let n_sequence_fields = Self::Data::REQUIRED_FIELD_NAMES.len() + n_unnamed_fields;
        let n_fields = n_sequence_fields + Self::Data::OPTIONAL_FIELD_NAMES.len();
        class.set_attr(
            identifier!(ctx, n_sequence_fields),
            ctx.new_int(n_sequence_fields).into(),
        );
        class.set_attr(identifier!(ctx, n_fields), ctx.new_int(n_fields).into());
        class.set_attr(
            identifier!(ctx, n_unnamed_fields),
            ctx.new_int(n_unnamed_fields).into(),
        );

        // Override as_sequence and as_mapping slots to use visible length
        class
            .slots
            .as_sequence
            .copy_from(&STRUCT_SEQUENCE_AS_SEQUENCE);
        class
            .slots
            .as_mapping
            .copy_from(&STRUCT_SEQUENCE_AS_MAPPING);

        // Override iter slot to return only visible elements
        class.slots.iter.store(Some(struct_sequence_iter));

        // Override hash slot to hash only visible elements
        class.slots.hash.store(Some(struct_sequence_hash));

        // Override richcompare slot to compare only visible elements
        class
            .slots
            .richcompare
            .store(Some(struct_sequence_richcompare));

        // Default __reduce__: only set if not already overridden by the impl's extend_class.
        // This allows struct sequences like sched_param to provide a custom __reduce__
        // (equivalent to METH_COEXIST in structseq.c).
        if !class
            .attributes
            .read()
            .contains_key(ctx.intern_str("__reduce__"))
        {
            class.set_attr(
                ctx.intern_str("__reduce__"),
                DEFAULT_STRUCTSEQ_REDUCE.to_proper_method(class, ctx),
            );
        }
    }
}

/// Iterator function for struct sequences - returns only visible elements
fn struct_sequence_iter(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let tuple = zelf
        .downcast_ref::<PyTuple>()
        .ok_or_else(|| vm.new_type_error("expected tuple"))?;
    let n_seq = get_visible_len(&zelf, vm)?;
    let visible: Vec<_> = tuple.iter().take(n_seq).cloned().collect();
    let visible_tuple = PyTuple::new_ref(visible, &vm.ctx);
    visible_tuple
        .as_object()
        .to_owned()
        .get_iter(vm)
        .map(Into::into)
}

/// Hash function for struct sequences - hashes only visible elements
fn struct_sequence_hash(
    zelf: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<crate::common::hash::PyHash> {
    let tuple = zelf
        .downcast_ref::<PyTuple>()
        .ok_or_else(|| vm.new_type_error("expected tuple"))?;
    let n_seq = get_visible_len(zelf, vm)?;
    // Create a visible-only tuple and hash it
    let visible: Vec<_> = tuple.iter().take(n_seq).cloned().collect();
    let visible_tuple = PyTuple::new_ref(visible, &vm.ctx);
    visible_tuple.as_object().hash(vm)
}

/// Rich comparison for struct sequences - compares only visible elements
fn struct_sequence_richcompare(
    zelf: &PyObject,
    other: &PyObject,
    op: PyComparisonOp,
    vm: &VirtualMachine,
) -> PyResult<Either<PyObjectRef, PyComparisonValue>> {
    let zelf_tuple = zelf
        .downcast_ref::<PyTuple>()
        .ok_or_else(|| vm.new_type_error("expected tuple"))?;

    // If other is not a tuple, return NotImplemented
    let Some(other_tuple) = other.downcast_ref::<PyTuple>() else {
        return Ok(Either::B(PyComparisonValue::NotImplemented));
    };

    let zelf_len = get_visible_len(zelf, vm)?;
    // For other, try to get visible len; if it fails (not a struct sequence), use full length
    let other_len = get_visible_len(other, vm).unwrap_or(other_tuple.len());

    let zelf_visible = &zelf_tuple.as_slice()[..zelf_len];
    let other_visible = &other_tuple.as_slice()[..other_len];

    // Use the same comparison logic as regular tuples
    zelf_visible
        .iter()
        .richcompare(other_visible.iter(), op, vm)
        .map(|v| Either::B(PyComparisonValue::Implemented(v)))
}
