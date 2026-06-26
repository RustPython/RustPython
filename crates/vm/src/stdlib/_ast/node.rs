use crate::{PyObjectRef, PyResult, VirtualMachine, builtins::PyList};
use rustpython_compiler_core::SourceFile;
use thin_vec::ThinVec;

pub(crate) trait Node: Sized {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef;
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self>;

    /// Used in `Option::ast_from_object`; if `true`, that impl will return None.
    fn is_none(&self) -> bool {
        false
    }
}

impl<T: Node> Node for Vec<T> {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        vm.ctx
            .new_list(
                self.into_iter()
                    .map(|node| node.ast_to_object(vm, source_file))
                    .collect(),
            )
            .into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let list = object.downcast_ref::<PyList>().ok_or_else(|| {
            vm.new_type_error(format!(
                "AST list field must be a list, not a {}",
                object.class().name()
            ))
        })?;
        let len = list.borrow_vec().len();
        let mut result = Self::with_capacity(len);
        for i in 0..len {
            let item = {
                let items = list.borrow_vec();
                if items.len() != len {
                    return Err(
                        vm.new_runtime_error("AST list field changed size during iteration")
                    );
                }
                items[i].clone()
            };
            result.push(vm.with_recursion("while traversing AST node", || {
                Node::ast_from_object(vm, source_file, item)
            })?);
            if list.borrow_vec().len() != len {
                return Err(vm.new_runtime_error("AST list field changed size during iteration"));
            }
        }
        Ok(result)
    }
}

impl<T: Node> Node for ThinVec<T> {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        vm.ctx
            .new_list(
                self.into_iter()
                    .map(|node| node.ast_to_object(vm, source_file))
                    .collect(),
            )
            .into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Vec::<T>::ast_from_object(vm, source_file, object).map(Into::into)
    }
}

impl<T: Node> Node for Box<T> {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        (*self).ast_to_object(vm, source_file)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        vm.with_recursion("while traversing AST node", || {
            T::ast_from_object(vm, source_file, object).map(Self::new)
        })
    }

    fn is_none(&self) -> bool {
        (**self).is_none()
    }
}

impl<T: Node> Node for Option<T> {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        match self {
            Some(node) => node.ast_to_object(vm, source_file),
            None => vm.ctx.none(),
        }
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        if vm.is_none(&object) {
            Ok(None)
        } else {
            let x = T::ast_from_object(vm, source_file, object)?;
            Ok((!x.is_none()).then_some(x))
        }
    }
}

pub(super) struct BoxedSlice<T>(pub(super) Box<[T]>);

impl<T: Node> Node for BoxedSlice<T> {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        self.0.into_vec().ast_to_object(vm, source_file)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self(
            <Vec<T> as Node>::ast_from_object(vm, source_file, object)?.into_boxed_slice(),
        ))
    }
}
