use crate::{PyObjectRef, PyResult, VirtualMachine};
use rustpython_compiler_source::SourceCodeOwned;

pub(crate) trait Node: Sized {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef;
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self>;

    /// Used in `Option::ast_from_object`; if `true`, that impl will return None.
    fn is_none(&self) -> bool {
        false
    }
}

impl<T: Node> Node for Vec<T> {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        vm.ctx
            .new_list(
                self.into_iter()
                    .map(|node| node.ast_to_object(vm, source_code))
                    .collect(),
            )
            .into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        vm.extract_elements_with(&object, |obj| Node::ast_from_object(vm, source_code, obj))
    }
}

impl<T: Node> Node for Box<T> {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        (*self).ast_to_object(vm, source_code)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        T::ast_from_object(vm, source_code, object).map(Self::new)
    }

    fn is_none(&self) -> bool {
        (**self).is_none()
    }
}

impl<T: Node> Node for Option<T> {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        match self {
            Some(node) => node.ast_to_object(vm, source_code),
            None => vm.ctx.none(),
        }
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        if vm.is_none(&object) {
            Ok(None)
        } else {
            let x = T::ast_from_object(vm, source_code, object)?;
            Ok((!x.is_none()).then_some(x))
        }
    }
}

pub(super) struct BoxedSlice<T>(pub(super) Box<[T]>);

impl<T: Node> Node for BoxedSlice<T> {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        self.0.into_vec().ast_to_object(vm, source_code)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self(
            <Vec<T> as Node>::ast_from_object(vm, source_code, object)?.into_boxed_slice(),
        ))
    }
}
