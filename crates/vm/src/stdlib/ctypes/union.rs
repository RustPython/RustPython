use super::base::{CDATA_BUFFER_METHODS, StgInfoFlags};
use super::{PyCData, PyCField, StgInfo};
use crate::builtins::{PyList, PyStr, PyTuple, PyType, PyTypeRef};
use crate::convert::ToPyObject;
use crate::function::FuncArgs;
use crate::function::PySetterValue;
use crate::protocol::{BufferDescriptor, PyBuffer};
use crate::types::{AsBuffer, Constructor, Initializer, SetAttr};
use crate::{AsObject, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine};
use std::borrow::Cow;

/// Calculate Union type size from _fields_ (max field size)
pub(super) fn calculate_union_size(cls: &Py<PyType>, vm: &VirtualMachine) -> PyResult<usize> {
    if let Ok(fields_attr) = cls.as_object().get_attr("_fields_", vm) {
        let fields: Vec<PyObjectRef> = fields_attr.try_to_value(vm)?;
        let mut max_size = 0usize;

        for field in fields.iter() {
            if let Some(tuple) = field.downcast_ref::<PyTuple>()
                && let Some(field_type) = tuple.get(1)
            {
                let field_size = super::_ctypes::sizeof(field_type.clone(), vm)?;
                max_size = max_size.max(field_size);
            }
        }
        return Ok(max_size);
    }
    Ok(0)
}

/// PyCUnionType - metaclass for Union
#[pyclass(name = "UnionType", base = PyType, module = "_ctypes")]
#[derive(Debug)]
#[repr(transparent)]
pub(super) struct PyCUnionType(PyType);

impl Constructor for PyCUnionType {
    type Args = FuncArgs;

    fn slot_new(metatype: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // 1. Create the new class using PyType::slot_new
        let new_class = crate::builtins::PyType::slot_new(metatype, args, vm)?;

        // 2. Get the new type
        let new_type = new_class
            .clone()
            .downcast::<PyType>()
            .map_err(|_| vm.new_type_error("expected type"))?;

        // 3. Mark base classes as finalized (subclassing finalizes the parent)
        new_type.mark_bases_final();

        // 4. Initialize StgInfo for the new type (initialized=false, to be set in init)
        let stg_info = StgInfo::default();
        let _ = new_type.init_type_data(stg_info);

        // Note: _fields_ processing moved to Initializer::init()
        Ok(new_class)
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

impl Initializer for PyCUnionType {
    type Args = FuncArgs;

    fn init(zelf: crate::PyRef<Self>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        // Get the type as PyTypeRef by converting PyRef<Self> -> PyObjectRef -> PyRef<PyType>
        let obj: PyObjectRef = zelf.clone().into();
        let new_type: PyTypeRef = obj
            .downcast()
            .map_err(|_| vm.new_type_error("expected type"))?;

        // Check for _abstract_ attribute - skip initialization if present
        if new_type
            .get_direct_attr(vm.ctx.intern_str("_abstract_"))
            .is_some()
        {
            return Ok(());
        }

        new_type.check_not_initialized(vm)?;

        // Process _fields_ if defined directly on this class (not inherited)
        // Use set_attr to trigger setattro
        if let Some(fields_attr) = new_type.get_direct_attr(vm.ctx.intern_str("_fields_")) {
            new_type
                .as_object()
                .set_attr(vm.ctx.intern_str("_fields_"), fields_attr, vm)?;
        } else {
            // No _fields_ defined - try to copy from base class
            let (has_base_info, base_clone) = {
                let bases = new_type.bases.read();
                if let Some(base) = bases.first() {
                    (base.stg_info_opt().is_some(), Some(base.clone()))
                } else {
                    (false, None)
                }
            };

            if has_base_info && let Some(ref base) = base_clone {
                // Clone base StgInfo (release guard before getting mutable reference)
                let stg_info_opt = base.stg_info_opt().map(|baseinfo| {
                    let mut stg_info = baseinfo.clone();
                    stg_info.flags &= !StgInfoFlags::DICTFLAG_FINAL; // Clear FINAL flag in subclass
                    stg_info.initialized = true;
                    stg_info
                });

                if let Some(stg_info) = stg_info_opt {
                    // Mark base as FINAL (now guard is released)
                    if let Some(mut base_stg) = base.get_type_data_mut::<StgInfo>() {
                        base_stg.flags |= StgInfoFlags::DICTFLAG_FINAL;
                    }

                    super::base::set_or_init_stginfo(&new_type, stg_info);
                    return Ok(());
                }
            }

            // No base StgInfo - create default
            let mut stg_info = StgInfo::new(0, 1);
            stg_info.flags |= StgInfoFlags::TYPEFLAG_HASUNION;
            stg_info.paramfunc = super::base::ParamFunc::Union;
            // PEP 3118 doesn't support union. Use 'B' for bytes.
            stg_info.format = Some("B".to_string());
            super::base::set_or_init_stginfo(&new_type, stg_info);
        }

        Ok(())
    }
}

impl PyCUnionType {
    /// Process _fields_ and create CField descriptors
    /// For Union, all fields start at offset 0
    fn process_fields(
        cls: &Py<PyType>,
        fields_attr: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // Check if already finalized
        {
            let Some(stg_info) = cls.get_type_data::<StgInfo>() else {
                return Err(vm.new_type_error("ctypes state is not initialized"));
            };
            if stg_info.is_final() {
                return Err(vm.new_attribute_error("_fields_ is final"));
            }
        } // Read lock released here

        // Check if this is a swapped byte order union
        let is_swapped = cls.as_object().get_attr("_swappedbytes_", vm).is_ok();

        let fields: Vec<PyObjectRef> = if let Some(list) = fields_attr.downcast_ref::<PyList>() {
            list.borrow_vec().to_vec()
        } else if let Some(tuple) = fields_attr.downcast_ref::<PyTuple>() {
            tuple.to_vec()
        } else {
            return Err(vm.new_type_error("_fields_ must be a list or tuple"));
        };

        let pack = super::base::get_usize_attr(cls.as_object(), "_pack_", 0, vm)?;
        let forced_alignment =
            super::base::get_usize_attr(cls.as_object(), "_align_", 1, vm)?.max(1);

        // Initialize size, alignment, type flags, and ffi_field_types from base class
        // Note: Union fields always start at offset 0, but we inherit base size/align
        let (mut max_size, mut max_align, mut has_pointer, mut has_bitfield, mut ffi_field_types) = {
            let bases = cls.bases.read();
            if let Some(base) = bases.first()
                && let Some(baseinfo) = base.stg_info_opt()
            {
                (
                    baseinfo.size,
                    std::cmp::max(baseinfo.align, forced_alignment),
                    baseinfo.flags.contains(StgInfoFlags::TYPEFLAG_HASPOINTER),
                    baseinfo.flags.contains(StgInfoFlags::TYPEFLAG_HASBITFIELD),
                    baseinfo.ffi_field_types.clone(),
                )
            } else {
                (0, forced_alignment, false, false, Vec::new())
            }
        };

        for (index, field) in fields.iter().enumerate() {
            let field_tuple = field
                .downcast_ref::<PyTuple>()
                .ok_or_else(|| vm.new_type_error("_fields_ must contain tuples"))?;

            if field_tuple.len() < 2 {
                return Err(vm.new_type_error(
                    "_fields_ tuple must have at least 2 elements (name, type)".to_string(),
                ));
            }

            let name = field_tuple
                .first()
                .expect("len checked")
                .downcast_ref::<PyStr>()
                .ok_or_else(|| vm.new_type_error("field name must be a string"))?
                .to_string();

            let field_type = field_tuple.get(1).expect("len checked").clone();

            // For swapped byte order unions, validate field type supports byte swapping
            if is_swapped {
                super::base::check_other_endian_support(&field_type, vm)?;
            }

            let size = super::base::get_field_size(&field_type, vm)?;
            let field_align = super::base::get_field_align(&field_type, vm);

            // Calculate effective alignment
            let effective_align = if pack > 0 {
                std::cmp::min(pack, field_align)
            } else {
                field_align
            };

            max_size = max_size.max(size);
            max_align = max_align.max(effective_align);

            // Propagate type flags from field type (HASPOINTER, HASBITFIELD)
            if let Some(type_obj) = field_type.downcast_ref::<PyType>()
                && let Some(field_stg) = type_obj.stg_info_opt()
            {
                // HASPOINTER: propagate if field is pointer or contains pointer
                if field_stg.flags.intersects(
                    StgInfoFlags::TYPEFLAG_ISPOINTER | StgInfoFlags::TYPEFLAG_HASPOINTER,
                ) {
                    has_pointer = true;
                }
                // HASBITFIELD: propagate directly
                if field_stg.flags.contains(StgInfoFlags::TYPEFLAG_HASBITFIELD) {
                    has_bitfield = true;
                }
                // Collect FFI type for this field
                ffi_field_types.push(field_stg.to_ffi_type());
            }

            // Mark field type as finalized (using type as field finalizes it)
            if let Some(type_obj) = field_type.downcast_ref::<PyType>() {
                if let Some(mut stg_info) = type_obj.get_type_data_mut::<StgInfo>() {
                    stg_info.flags |= StgInfoFlags::DICTFLAG_FINAL;
                } else {
                    // Create StgInfo with FINAL flag if it doesn't exist
                    let mut stg_info = StgInfo::new(size, field_align);
                    stg_info.flags |= StgInfoFlags::DICTFLAG_FINAL;
                    let _ = type_obj.init_type_data(stg_info);
                }
            }

            // For Union, all fields start at offset 0
            let field_type_ref = field_type
                .clone()
                .downcast::<PyType>()
                .map_err(|_| vm.new_type_error("_fields_ type must be a ctypes type"))?;
            let c_field = PyCField::new(field_type_ref, 0, size as isize, index);

            cls.set_attr(vm.ctx.intern_str(name), c_field.to_pyobject(vm));
        }

        // Calculate total_align and aligned_size
        let total_align = std::cmp::max(max_align, forced_alignment);
        let aligned_size = if total_align > 0 {
            max_size.div_ceil(total_align) * total_align
        } else {
            max_size
        };

        // Store StgInfo with aligned size
        let mut stg_info = StgInfo::new(aligned_size, total_align);
        stg_info.flags |= StgInfoFlags::DICTFLAG_FINAL | StgInfoFlags::TYPEFLAG_HASUNION;
        // PEP 3118 doesn't support union. Use 'B' for bytes.
        stg_info.format = Some("B".to_string());
        if has_pointer {
            stg_info.flags |= StgInfoFlags::TYPEFLAG_HASPOINTER;
        }
        if has_bitfield {
            stg_info.flags |= StgInfoFlags::TYPEFLAG_HASBITFIELD;
        }
        stg_info.paramfunc = super::base::ParamFunc::Union;
        // Set byte order: swap if _swappedbytes_ is defined
        stg_info.big_endian = super::base::is_big_endian(is_swapped);
        // Store FFI field types for union passing
        stg_info.ffi_field_types = ffi_field_types;
        super::base::set_or_init_stginfo(cls, stg_info);

        // Process _anonymous_ fields
        super::base::make_anon_fields(cls, vm)?;

        Ok(())
    }
}

#[pyclass(flags(BASETYPE), with(Constructor, Initializer, SetAttr))]
impl PyCUnionType {
    #[pymethod]
    fn from_param(zelf: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // zelf is the union type class that from_param was called on
        let cls = zelf
            .downcast::<PyType>()
            .map_err(|_| vm.new_type_error("from_param: expected a type"))?;

        // 1. If already an instance of the requested type, return it
        if value.is_instance(cls.as_object(), vm)? {
            return Ok(value);
        }

        // 2. Check for CArgObject (PyCArg_CheckExact)
        if let Some(carg) = value.downcast_ref::<super::_ctypes::CArgObject>() {
            // Check against proto (for pointer types)
            if let Some(stg_info) = cls.stg_info_opt()
                && let Some(ref proto) = stg_info.proto
                && carg.obj.is_instance(proto.as_object(), vm)?
            {
                return Ok(value);
            }
            // Fallback: check if the wrapped object is an instance of the requested type
            if carg.obj.is_instance(cls.as_object(), vm)? {
                return Ok(value); // Return the CArgObject as-is
            }
            // CArgObject but wrong type
            return Err(vm.new_type_error(format!(
                "expected {} instance instead of pointer to {}",
                cls.name(),
                carg.obj.class().name()
            )));
        }

        // 3. Check for _as_parameter_ attribute
        if let Ok(as_parameter) = value.get_attr("_as_parameter_", vm) {
            return PyCUnionType::from_param(cls.as_object().to_owned(), as_parameter, vm);
        }

        Err(vm.new_type_error(format!(
            "expected {} instance instead of {}",
            cls.name(),
            value.class().name()
        )))
    }

    /// Called when a new Union subclass is created
    #[pyclassmethod]
    fn __init_subclass__(cls: PyTypeRef, vm: &VirtualMachine) -> PyResult<()> {
        cls.mark_bases_final();

        // Check if _fields_ is defined
        if let Some(fields_attr) = cls.get_direct_attr(vm.ctx.intern_str("_fields_")) {
            Self::process_fields(&cls, fields_attr, vm)?;
        }
        Ok(())
    }
}

impl SetAttr for PyCUnionType {
    fn setattro(
        zelf: &Py<Self>,
        attr_name: &Py<PyStr>,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let pytype: &Py<PyType> = zelf.to_base();
        let attr_name_interned = vm.ctx.intern_str(attr_name.as_str());

        // 1. First, do PyType's setattro (PyType_Type.tp_setattro first)
        // Check for data descriptor first
        if let Some(attr) = pytype.get_class_attr(attr_name_interned) {
            let descr_set = attr.class().slots.descr_set.load();
            if let Some(descriptor) = descr_set {
                descriptor(&attr, pytype.to_owned().into(), value.clone(), vm)?;
                // After successful setattro, check if _fields_ and call process_fields
                if attr_name.as_str() == "_fields_"
                    && let PySetterValue::Assign(fields_value) = value
                {
                    PyCUnionType::process_fields(pytype, fields_value, vm)?;
                }
                return Ok(());
            }
        }

        // Store in type's attributes dict
        match &value {
            PySetterValue::Assign(v) => {
                pytype
                    .attributes
                    .write()
                    .insert(attr_name_interned, v.clone());
            }
            PySetterValue::Delete => {
                let prev = pytype.attributes.write().shift_remove(attr_name_interned);
                if prev.is_none() {
                    return Err(vm.new_attribute_error(format!(
                        "type object '{}' has no attribute '{}'",
                        pytype.name(),
                        attr_name.as_str(),
                    )));
                }
            }
        }

        // 2. If _fields_, call process_fields (which checks FINAL internally)
        if attr_name.as_str() == "_fields_"
            && let PySetterValue::Assign(fields_value) = value
        {
            PyCUnionType::process_fields(pytype, fields_value, vm)?;
        }

        Ok(())
    }
}

/// PyCUnion - base class for Union
#[pyclass(module = "_ctypes", name = "Union", base = PyCData, metaclass = "PyCUnionType")]
#[repr(transparent)]
pub struct PyCUnion(pub PyCData);

impl std::fmt::Debug for PyCUnion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyCUnion")
            .field("size", &self.0.size())
            .finish()
    }
}

impl Constructor for PyCUnion {
    type Args = FuncArgs;

    fn slot_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // Check for abstract class and extract values in a block to drop the borrow
        let (total_size, total_align) = {
            let stg_info = cls.stg_info(vm)?;
            (stg_info.size, stg_info.align)
        };

        // Mark the class as finalized (instance creation finalizes the type)
        if let Some(mut stg_info_mut) = cls.get_type_data_mut::<StgInfo>() {
            stg_info_mut.flags |= StgInfoFlags::DICTFLAG_FINAL;
        }

        // Initialize buffer with zeros using computed size
        let new_stg_info = StgInfo::new(total_size, total_align);
        PyCUnion(PyCData::from_stg_info(&new_stg_info))
            .into_ref_with_type(vm, cls)
            .map(Into::into)
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

impl PyCUnion {
    /// Recursively initialize positional arguments through inheritance chain
    /// Returns the number of arguments consumed
    fn init_pos_args(
        self_obj: &Py<Self>,
        type_obj: &Py<PyType>,
        args: &[PyObjectRef],
        kwargs: &indexmap::IndexMap<String, PyObjectRef>,
        index: usize,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let mut current_index = index;

        // 1. First process base class fields recursively
        // Recurse if base has StgInfo
        let base_clone = {
            let bases = type_obj.bases.read();
            if let Some(base) = bases.first() &&
                // Check if base has StgInfo
                base.stg_info_opt().is_some()
            {
                Some(base.clone())
            } else {
                None
            }
        };

        if let Some(ref base) = base_clone {
            current_index = Self::init_pos_args(self_obj, base, args, kwargs, current_index, vm)?;
        }

        // 2. Process this class's _fields_
        if let Some(fields_attr) = type_obj.get_direct_attr(vm.ctx.intern_str("_fields_")) {
            let fields: Vec<PyObjectRef> = fields_attr.try_to_value(vm)?;

            for field in fields.iter() {
                if current_index >= args.len() {
                    break;
                }
                if let Some(tuple) = field.downcast_ref::<PyTuple>()
                    && let Some(name) = tuple.first()
                    && let Some(name_str) = name.downcast_ref::<PyStr>()
                {
                    let field_name = name_str.as_str().to_owned();
                    // Check for duplicate in kwargs
                    if kwargs.contains_key(&field_name) {
                        return Err(vm.new_type_error(format!(
                            "duplicate values for field {:?}",
                            field_name
                        )));
                    }
                    self_obj.as_object().set_attr(
                        vm.ctx.intern_str(field_name),
                        args[current_index].clone(),
                        vm,
                    )?;
                    current_index += 1;
                }
            }
        }

        Ok(current_index)
    }
}

impl Initializer for PyCUnion {
    type Args = FuncArgs;

    fn init(zelf: crate::PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        // Struct_init: handle positional and keyword arguments
        let cls = zelf.class().to_owned();

        // 1. Process positional arguments recursively through inheritance chain
        if !args.args.is_empty() {
            let consumed = PyCUnion::init_pos_args(&zelf, &cls, &args.args, &args.kwargs, 0, vm)?;

            if consumed < args.args.len() {
                return Err(vm.new_type_error("too many initializers"));
            }
        }

        // 2. Process keyword arguments
        for (key, value) in args.kwargs.iter() {
            zelf.as_object()
                .set_attr(vm.ctx.intern_str(key.as_str()), value.clone(), vm)?;
        }

        Ok(())
    }
}

#[pyclass(
    flags(BASETYPE, IMMUTABLETYPE),
    with(Constructor, Initializer, AsBuffer)
)]
impl PyCUnion {}

impl AsBuffer for PyCUnion {
    fn as_buffer(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
        let buffer_len = zelf.0.buffer.read().len();

        // PyCData_NewGetBuffer: use info->format if available, otherwise "B"
        let format = zelf
            .class()
            .stg_info_opt()
            .and_then(|info| info.format.clone())
            .unwrap_or_else(|| "B".to_string());

        // Union: ndim=0, shape=(), itemsize=union_size
        let buf = PyBuffer::new(
            zelf.to_owned().into(),
            BufferDescriptor {
                len: buffer_len,
                readonly: false,
                itemsize: buffer_len,
                format: Cow::Owned(format),
                dim_desc: vec![], // ndim=0 means empty dim_desc
            },
            &CDATA_BUFFER_METHODS,
        );
        Ok(buf)
    }
}
