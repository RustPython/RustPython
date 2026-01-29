//! Implementation of the `_types` module.
//!
//! This module exposes built-in types that are used by the `types` module.

pub(crate) use _types::module_def;

#[pymodule]
#[allow(non_snake_case)]
mod _types {
    use crate::{PyObjectRef, VirtualMachine};

    #[pyattr]
    fn AsyncGeneratorType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.async_generator.to_owned().into()
    }

    #[pyattr]
    fn BuiltinFunctionType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx
            .types
            .builtin_function_or_method_type
            .to_owned()
            .into()
    }

    #[pyattr]
    fn BuiltinMethodType(vm: &VirtualMachine) -> PyObjectRef {
        // Same as BuiltinFunctionType in CPython
        vm.ctx
            .types
            .builtin_function_or_method_type
            .to_owned()
            .into()
    }

    #[pyattr]
    fn CapsuleType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.capsule_type.to_owned().into()
    }

    #[pyattr]
    fn CellType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.cell_type.to_owned().into()
    }

    #[pyattr]
    fn CodeType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.code_type.to_owned().into()
    }

    #[pyattr]
    fn CoroutineType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.coroutine_type.to_owned().into()
    }

    #[pyattr]
    fn EllipsisType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.ellipsis_type.to_owned().into()
    }

    #[pyattr]
    fn FrameType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.frame_type.to_owned().into()
    }

    #[pyattr]
    fn FunctionType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.function_type.to_owned().into()
    }

    #[pyattr]
    fn GeneratorType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.generator_type.to_owned().into()
    }

    #[pyattr]
    fn GenericAlias(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.generic_alias_type.to_owned().into()
    }

    #[pyattr]
    fn GetSetDescriptorType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.getset_type.to_owned().into()
    }

    #[pyattr]
    fn LambdaType(vm: &VirtualMachine) -> PyObjectRef {
        // Same as FunctionType in CPython
        vm.ctx.types.function_type.to_owned().into()
    }

    #[pyattr]
    fn MappingProxyType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.mappingproxy_type.to_owned().into()
    }

    #[pyattr]
    fn MemberDescriptorType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.member_descriptor_type.to_owned().into()
    }

    #[pyattr]
    fn MethodDescriptorType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.method_descriptor_type.to_owned().into()
    }

    #[pyattr]
    fn ClassMethodDescriptorType(vm: &VirtualMachine) -> PyObjectRef {
        // TODO: implement as separate type
        vm.ctx.types.method_descriptor_type.to_owned().into()
    }

    #[pyattr]
    fn MethodType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.bound_method_type.to_owned().into()
    }

    #[pyattr]
    fn MethodWrapperType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.method_wrapper_type.to_owned().into()
    }

    #[pyattr]
    fn ModuleType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.module_type.to_owned().into()
    }

    #[pyattr]
    fn NoneType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.none_type.to_owned().into()
    }

    #[pyattr]
    fn NotImplementedType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.not_implemented_type.to_owned().into()
    }

    #[pyattr]
    fn SimpleNamespace(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.namespace_type.to_owned().into()
    }

    #[pyattr]
    fn TracebackType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.traceback_type.to_owned().into()
    }

    #[pyattr]
    fn UnionType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.union_type.to_owned().into()
    }

    #[pyattr]
    fn WrapperDescriptorType(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.types.wrapper_descriptor_type.to_owned().into()
    }
}
