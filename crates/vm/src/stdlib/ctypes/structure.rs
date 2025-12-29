use super::base::{CDATA_BUFFER_METHODS, PyCData, PyCField, StgInfo, StgInfoFlags};
use crate::builtins::{PyList, PyStr, PyTuple, PyType, PyTypeRef};
use crate::convert::ToPyObject;
use crate::function::FuncArgs;
use crate::function::PySetterValue;
use crate::protocol::{BufferDescriptor, PyBuffer, PyNumberMethods};
use crate::types::{AsBuffer, AsNumber, Constructor, Initializer, SetAttr};
use crate::{AsObject, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine};
use alloc::borrow::Cow;
use core::fmt::Debug;
use num_traits::ToPrimitive;

/// Calculate Structure type size from _fields_ (sum of field sizes)
pub(super) fn calculate_struct_size(cls: &Py<PyType>, vm: &VirtualMachine) -> PyResult<usize> {
    if let Ok(fields_attr) = cls.as_object().get_attr("_fields_", vm) {
        let fields: Vec<PyObjectRef> = fields_attr.try_to_value(vm)?;
        let mut total_size = 0usize;

        for field in fields.iter() {
            if let Some(tuple) = field.downcast_ref::<PyTuple>()
                && let Some(field_type) = tuple.get(1)
            {
                total_size += super::_ctypes::sizeof(field_type.clone(), vm)?;
            }
        }
        return Ok(total_size);
    }
    Ok(0)
}

/// PyCStructType - metaclass for Structure
#[pyclass(name = "PyCStructType", base = PyType, module = "_ctypes")]
#[derive(Debug)]
#[repr(transparent)]
pub(super) struct PyCStructType(PyType);

impl Constructor for PyCStructType {
    type Args = FuncArgs;

    fn slot_new(metatype: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // 1. Create the new class using PyType::slot_new
        let new_class = crate::builtins::type_::PyType::slot_new(metatype, args, vm)?;

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

impl Initializer for PyCStructType {
    type Args = FuncArgs;

    fn init(zelf: crate::PyRef<Self>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        // Get the type as PyTypeRef by converting PyRef<Self> -> PyObjectRef -> PyRef<PyType>
        let obj: PyObjectRef = zelf.clone().into();
        let new_type: PyTypeRef = obj
            .downcast()
            .map_err(|_| vm.new_type_error("expected type"))?;

        // Backward compatibility: skip initialization for abstract types
        if new_type
            .get_direct_attr(vm.ctx.intern_str("_abstract_"))
            .is_some()
        {
            return Ok(());
        }

        new_type.check_not_initialized(vm)?;

        // Process _fields_ if defined directly on this class (not inherited)
        if let Some(fields_attr) = new_type.get_direct_attr(vm.ctx.intern_str("_fields_")) {
            Self::process_fields(&new_type, fields_attr, vm)?;
        } else {
            // No _fields_ defined - try to copy from base class (PyCStgInfo_clone)
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
                    stg_info.flags &= !StgInfoFlags::DICTFLAG_FINAL; // Clear FINAL in subclass
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
            stg_info.paramfunc = super::base::ParamFunc::Structure;
            stg_info.format = Some("B".to_string());
            super::base::set_or_init_stginfo(&new_type, stg_info);
        }

        Ok(())
    }
}

#[pyclass(flags(BASETYPE), with(AsNumber, Constructor, Initializer, SetAttr))]
impl PyCStructType {
    #[pymethod]
    fn from_param(zelf: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // zelf is the structure type class that from_param was called on
        let cls = zelf
            .downcast::<PyType>()
            .map_err(|_| vm.new_type_error("from_param: expected a type"))?;

        // 1. If already an instance of the requested type, return it
        if value.is_instance(cls.as_object(), vm)? {
            return Ok(value);
        }

        // 2. Check for _as_parameter_ attribute
        if let Ok(as_parameter) = value.get_attr("_as_parameter_", vm) {
            return PyCStructType::from_param(cls.as_object().to_owned(), as_parameter, vm);
        }

        Err(vm.new_type_error(format!(
            "expected {} instance instead of {}",
            cls.name(),
            value.class().name()
        )))
    }

    /// Called when a new Structure subclass is created
    #[pyclassmethod]
    fn __init_subclass__(cls: PyTypeRef, vm: &VirtualMachine) -> PyResult<()> {
        cls.mark_bases_final();

        // Check if _fields_ is defined
        if let Some(fields_attr) = cls.get_direct_attr(vm.ctx.intern_str("_fields_")) {
            Self::process_fields(&cls, fields_attr, vm)?;
        }
        Ok(())
    }

    /// Process _fields_ and create CField descriptors
    fn process_fields(
        cls: &Py<PyType>,
        fields_attr: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // Check if this is a swapped byte order structure
        let is_swapped = cls.as_object().get_attr("_swappedbytes_", vm).is_ok();

        // Try to downcast to list or tuple
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

        // Determine byte order for format string
        let big_endian = super::base::is_big_endian(is_swapped);

        // Initialize offset, alignment, type flags, and ffi_field_types from base class
        let (
            mut offset,
            mut max_align,
            mut has_pointer,
            mut has_union,
            mut has_bitfield,
            mut ffi_field_types,
        ) = {
            let bases = cls.bases.read();
            if let Some(base) = bases.first()
                && let Some(baseinfo) = base.stg_info_opt()
            {
                (
                    baseinfo.size,
                    core::cmp::max(baseinfo.align, forced_alignment),
                    baseinfo.flags.contains(StgInfoFlags::TYPEFLAG_HASPOINTER),
                    baseinfo.flags.contains(StgInfoFlags::TYPEFLAG_HASUNION),
                    baseinfo.flags.contains(StgInfoFlags::TYPEFLAG_HASBITFIELD),
                    baseinfo.ffi_field_types.clone(),
                )
            } else {
                (0, forced_alignment, false, false, false, Vec::new())
            }
        };

        // Initialize PEP3118 format string
        let mut format = String::from("T{");
        let mut last_end = 0usize; // Track end of last field for padding calculation

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

            // For swapped byte order structures, validate field type supports byte swapping
            if is_swapped {
                super::base::check_other_endian_support(&field_type, vm)?;
            }

            // Get size and alignment of the field type
            let size = super::base::get_field_size(&field_type, vm)?;
            let field_align = super::base::get_field_align(&field_type, vm);

            // Calculate effective alignment (PyCField_FromDesc)
            let effective_align = if pack > 0 {
                core::cmp::min(pack, field_align)
            } else {
                field_align
            };

            // Apply padding to align offset (cfield.c NO_BITFIELD case)
            if effective_align > 0 && offset % effective_align != 0 {
                let delta = effective_align - (offset % effective_align);
                offset += delta;
            }

            max_align = max_align.max(effective_align);

            // Propagate type flags from field type (HASPOINTER, HASUNION, HASBITFIELD)
            if let Some(type_obj) = field_type.downcast_ref::<PyType>()
                && let Some(field_stg) = type_obj.stg_info_opt()
            {
                // HASPOINTER: propagate if field is pointer or contains pointer
                if field_stg.flags.intersects(
                    StgInfoFlags::TYPEFLAG_ISPOINTER | StgInfoFlags::TYPEFLAG_HASPOINTER,
                ) {
                    has_pointer = true;
                }
                // HASUNION, HASBITFIELD: propagate directly
                if field_stg.flags.contains(StgInfoFlags::TYPEFLAG_HASUNION) {
                    has_union = true;
                }
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

            // Build format string: add padding before field
            let padding = offset - last_end;
            if padding > 0 {
                if padding != 1 {
                    format.push_str(&padding.to_string());
                }
                format.push('x');
            }

            // Get field format and add to format string
            let field_format = super::base::get_field_format(&field_type, big_endian, vm);

            // Handle arrays: prepend shape
            if let Some(type_obj) = field_type.downcast_ref::<PyType>()
                && let Some(field_stg) = type_obj.stg_info_opt()
                && !field_stg.shape.is_empty()
            {
                let shape_str = field_stg
                    .shape
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                format.push_str(&std::format!("({}){}", shape_str, field_format));
            } else {
                format.push_str(&field_format);
            }

            // Add field name
            format.push(':');
            format.push_str(&name);
            format.push(':');

            // Create CField descriptor with padding-adjusted offset
            let field_type_ref = field_type
                .clone()
                .downcast::<PyType>()
                .map_err(|_| vm.new_type_error("_fields_ type must be a ctypes type"))?;
            let c_field = PyCField::new(field_type_ref, offset as isize, size as isize, index);

            // Set the CField as a class attribute
            cls.set_attr(vm.ctx.intern_str(name.clone()), c_field.to_pyobject(vm));

            // Update tracking
            last_end = offset + size;
            offset += size;
        }

        // Calculate total_align = max(max_align, forced_alignment)
        let total_align = core::cmp::max(max_align, forced_alignment);

        // Calculate aligned_size (PyCStructUnionType_update_stginfo)
        let aligned_size = if total_align > 0 {
            offset.div_ceil(total_align) * total_align
        } else {
            offset
        };

        // Complete format string: add final padding and close
        let final_padding = aligned_size - last_end;
        if final_padding > 0 {
            if final_padding != 1 {
                format.push_str(&final_padding.to_string());
            }
            format.push('x');
        }
        format.push('}');

        // Store StgInfo with aligned size and total alignment
        let mut stg_info = StgInfo::new(aligned_size, total_align);
        stg_info.format = Some(format);
        stg_info.flags |= StgInfoFlags::DICTFLAG_FINAL; // Mark as finalized
        if has_pointer {
            stg_info.flags |= StgInfoFlags::TYPEFLAG_HASPOINTER;
        }
        if has_union {
            stg_info.flags |= StgInfoFlags::TYPEFLAG_HASUNION;
        }
        if has_bitfield {
            stg_info.flags |= StgInfoFlags::TYPEFLAG_HASBITFIELD;
        }
        stg_info.paramfunc = super::base::ParamFunc::Structure;
        // Set byte order: swap if _swappedbytes_ is defined
        stg_info.big_endian = super::base::is_big_endian(is_swapped);
        // Store FFI field types for structure passing
        stg_info.ffi_field_types = ffi_field_types;
        super::base::set_or_init_stginfo(cls, stg_info);

        // Process _anonymous_ fields
        super::base::make_anon_fields(cls, vm)?;

        Ok(())
    }

    #[pymethod]
    fn __mul__(cls: PyTypeRef, n: isize, vm: &VirtualMachine) -> PyResult {
        use super::array::array_type_from_ctype;

        if n < 0 {
            return Err(vm.new_value_error(format!("Array length must be >= 0, not {n}")));
        }
        // Use cached array type creation
        array_type_from_ctype(cls.into(), n as usize, vm)
    }
}

impl AsNumber for PyCStructType {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            multiply: Some(|a, b, vm| {
                let cls = a
                    .downcast_ref::<PyType>()
                    .ok_or_else(|| vm.new_type_error("expected type"))?;
                let n = b
                    .try_index(vm)?
                    .as_bigint()
                    .to_isize()
                    .ok_or_else(|| vm.new_overflow_error("array size too large"))?;
                PyCStructType::__mul__(cls.to_owned(), n, vm)
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

impl SetAttr for PyCStructType {
    fn setattro(
        zelf: &Py<Self>,
        attr_name: &Py<PyStr>,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // Check if _fields_ is being set
        if attr_name.as_str() == "_fields_" {
            let pytype: &Py<PyType> = zelf.to_base();

            // Check finalization in separate scope to release read lock before process_fields
            // This prevents deadlock: process_fields needs write lock on the same RwLock
            let is_final = {
                let Some(stg_info) = pytype.get_type_data::<StgInfo>() else {
                    return Err(vm.new_type_error("ctypes state is not initialized"));
                };
                stg_info.is_final()
            }; // Read lock released here

            if is_final {
                return Err(vm.new_attribute_error("_fields_ is final"));
            }

            // Process _fields_ and set attribute
            let PySetterValue::Assign(fields_value) = value else {
                return Err(vm.new_attribute_error("cannot delete _fields_"));
            };
            // Process fields (this will also set DICTFLAG_FINAL)
            PyCStructType::process_fields(pytype, fields_value.clone(), vm)?;
            // Set the _fields_ attribute on the type
            pytype
                .attributes
                .write()
                .insert(vm.ctx.intern_str("_fields_"), fields_value);
            return Ok(());
        }
        // Delegate to PyType's setattro logic for type attributes
        let attr_name_interned = vm.ctx.intern_str(attr_name.as_str());
        let pytype: &Py<PyType> = zelf.to_base();

        // Check for data descriptor first
        if let Some(attr) = pytype.get_class_attr(attr_name_interned) {
            let descr_set = attr.class().slots.descr_set.load();
            if let Some(descriptor) = descr_set {
                return descriptor(&attr, pytype.to_owned().into(), value, vm);
            }
        }

        // Store in type's attributes dict
        if let PySetterValue::Assign(value) = value {
            pytype.attributes.write().insert(attr_name_interned, value);
        } else {
            let prev = pytype.attributes.write().shift_remove(attr_name_interned);
            if prev.is_none() {
                return Err(vm.new_attribute_error(format!(
                    "type object '{}' has no attribute '{}'",
                    pytype.name(),
                    attr_name.as_str(),
                )));
            }
        }
        Ok(())
    }
}

/// PyCStructure - base class for Structure instances
#[pyclass(
    module = "_ctypes",
    name = "Structure",
    base = PyCData,
    metaclass = "PyCStructType"
)]
#[repr(transparent)]
pub struct PyCStructure(pub PyCData);

impl Debug for PyCStructure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PyCStructure")
            .field("size", &self.0.size())
            .finish()
    }
}

impl Constructor for PyCStructure {
    type Args = FuncArgs;

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // Check for abstract class and extract values in a block to drop the borrow
        let (total_size, total_align, length) = {
            let stg_info = cls.stg_info(vm)?;
            (stg_info.size, stg_info.align, stg_info.length)
        };

        // Mark the class as finalized (instance creation finalizes the type)
        if let Some(mut stg_info_mut) = cls.get_type_data_mut::<StgInfo>() {
            stg_info_mut.flags |= StgInfoFlags::DICTFLAG_FINAL;
        }

        // Get _fields_ from the class using get_attr to properly search MRO
        let fields_attr = cls.as_object().get_attr("_fields_", vm).ok();

        // Collect field names for initialization
        let mut field_names: Vec<String> = Vec::new();
        if let Some(fields_attr) = fields_attr {
            let fields: Vec<PyObjectRef> = if let Some(list) = fields_attr.downcast_ref::<PyList>()
            {
                list.borrow_vec().to_vec()
            } else if let Some(tuple) = fields_attr.downcast_ref::<PyTuple>() {
                tuple.to_vec()
            } else {
                vec![]
            };

            for field in fields.iter() {
                let Some(field_tuple) = field.downcast_ref::<PyTuple>() else {
                    continue;
                };
                if field_tuple.len() < 2 {
                    continue;
                }
                if let Some(name) = field_tuple.first().unwrap().downcast_ref::<PyStr>() {
                    field_names.push(name.to_string());
                }
            }
        }

        // Initialize buffer with zeros using computed size
        let mut stg_info = StgInfo::new(total_size, total_align);
        stg_info.length = if length > 0 {
            length
        } else {
            field_names.len()
        };
        stg_info.paramfunc = super::base::ParamFunc::Structure;
        let instance = PyCStructure(PyCData::from_stg_info(&stg_info));

        // Handle keyword arguments for field initialization
        let py_instance = instance.into_ref_with_type(vm, cls.clone())?;
        let py_obj: PyObjectRef = py_instance.clone().into();

        // Set field values from kwargs using standard attribute setting
        for (key, value) in args.kwargs.iter() {
            if field_names.iter().any(|n| n == key.as_str()) {
                py_obj.set_attr(vm.ctx.intern_str(key.as_str()), value.clone(), vm)?;
            }
        }

        // Set field values from positional args
        if args.args.len() > field_names.len() {
            return Err(vm.new_type_error("too many initializers".to_string()));
        }
        for (i, value) in args.args.iter().enumerate() {
            py_obj.set_attr(
                vm.ctx.intern_str(field_names[i].as_str()),
                value.clone(),
                vm,
            )?;
        }

        Ok(py_instance.into())
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

// Note: GetAttr and SetAttr are not implemented here.
// Field access is handled by CField descriptors registered on the class.

#[pyclass(flags(BASETYPE, IMMUTABLETYPE), with(Constructor, AsBuffer))]
impl PyCStructure {
    #[pygetset]
    fn _b0_(&self) -> Option<PyObjectRef> {
        self.0.base.read().clone()
    }
}

impl AsBuffer for PyCStructure {
    fn as_buffer(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
        let buffer_len = zelf.0.buffer.read().len();

        // PyCData_NewGetBuffer: use info->format if available, otherwise "B"
        let format = zelf
            .class()
            .stg_info_opt()
            .and_then(|info| info.format.clone())
            .unwrap_or_else(|| "B".to_string());

        // Structure: ndim=0, shape=(), itemsize=struct_size
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
